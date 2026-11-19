//! Port of `lib/graph/ConcurrentCollectionWrapper.java`.

/// Wraps collection implementation. Automatically switches from one to another implementation
/// depending on the number of storing elements.
///
/// Effective collection implementation depends on the size of collection:
/// - for 1: singleton immutable List;
/// - for [2..6]: ArrayList;
/// - for [7...): CompactHashSet.
///
/// PORT: the iteration order is what reaches query output. A singleton list and an `ArrayList`
/// iterate in insertion order and remove by shifting; Guava's `CompactHashSet` iterates its entry
/// array (insertion order) and removes by moving the last entry into the freed slot, which is
/// `Vec::swap_remove`. The wrapper keeps the elements in one `Vec` and remembers which
/// representation Java would be using. Only single-threaded use is ported (`synchronized` is
/// implied by `&mut self`).
// Java: com/google/devtools/build/lib/graph/ConcurrentCollectionWrapper.java:36 ConcurrentCollectionWrapper
#[derive(Clone, Debug, Default)]
pub(crate) struct ConcurrentCollectionWrapper<T> {
    collection: Vec<T>,
    /// Java's `collection` is a `CompactHashSet`.
    is_hash_set: bool,
}

// Java: com/google/devtools/build/lib/graph/ConcurrentCollectionWrapper.java:38 ConcurrentCollectionWrapper.ARRAYLIST_THRESHOLD
const ARRAYLIST_THRESHOLD: usize = 6;

impl<T: Copy + PartialEq> ConcurrentCollectionWrapper<T> {
    pub(crate) fn new() -> Self {
        ConcurrentCollectionWrapper {
            collection: Vec::new(),
            is_hash_set: false,
        }
    }

    /// Returns the elements (Java: an unmodifiable view).
    // Java: com/google/devtools/build/lib/graph/ConcurrentCollectionWrapper.java:58 ConcurrentCollectionWrapper.get
    pub(crate) fn get(&self) -> &[T] {
        &self.collection
    }

    // Java: com/google/devtools/build/lib/graph/ConcurrentCollectionWrapper.java:65 ConcurrentCollectionWrapper.clear
    pub(crate) fn clear(&mut self) -> Vec<T> {
        self.is_hash_set = false;
        std::mem::take(&mut self.collection)
    }

    // Java: com/google/devtools/build/lib/graph/ConcurrentCollectionWrapper.java:71 ConcurrentCollectionWrapper.size
    pub(crate) fn size(&self) -> usize {
        self.collection.len()
    }

    /// Adds 'value' to wrapped collection. Replacing this collection instance for CompactHashSet
    /// from ArrayList.
    ///
    /// Returns `true` if the collection was modified; `false` if the collection was not modified.
    // Java: com/google/devtools/build/lib/graph/ConcurrentCollectionWrapper.java:83 ConcurrentCollectionWrapper.add
    pub(crate) fn add(&mut self, value: T) -> bool {
        if self.collection.is_empty() {
            // null -> SingletonList
            self.collection.push(value);
            return true;
        }
        if self.collection.contains(&value) {
            // already exists in this collection
            return false;
        }
        let previous_size = self.collection.len();
        if previous_size == ARRAYLIST_THRESHOLD {
            // ArrayList -> CompactHashSet: addAll(collection) then add(value) keeps the order.
            self.is_hash_set = true;
        }
        // SingletonList -> ArrayList, ArrayList, or HashSet: all append.
        self.collection.push(value);
        true
    }

    /// Removes 'value' from wrapped collection. Replacing this collection instance for ArrayList
    /// from CompactHashSet.
    ///
    /// Returns `true` if the collection was modified; `false` if the set collection not modified.
    // Java: com/google/devtools/build/lib/graph/ConcurrentCollectionWrapper.java:126 ConcurrentCollectionWrapper.remove
    pub(crate) fn remove(&mut self, value: T) -> bool {
        if self.collection.is_empty() {
            // null
            return false;
        }
        let previous_size = self.collection.len();
        if previous_size == 1 {
            if self.collection[0] == value {
                // -> null
                self.collection.clear();
                self.is_hash_set = false;
                return true;
            } else {
                return false;
            }
        }
        // now remove the value
        let Some(index) = self.collection.iter().position(|v| *v == value) else {
            return false;
        };
        if self.is_hash_set {
            // CompactHashSet.remove moves the last entry into the freed slot.
            self.collection.swap_remove(index);
        } else {
            self.collection.remove(index);
        }
        // may need to change representation
        if previous_size == 2 || previous_size == 1 + ARRAYLIST_THRESHOLD {
            // -> SingletonList, or -> ArrayList (addAll keeps the set's order)
            self.is_hash_set = false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_removal_shifts_and_set_removal_swaps() {
        let mut w = ConcurrentCollectionWrapper::new();
        for i in 0..8 {
            assert!(w.add(i));
        }
        assert!(!w.add(3));
        // CompactHashSet: 8 elements; removing 1 moves 7 into its slot.
        assert!(w.remove(1));
        assert_eq!(w.get(), &[0, 7, 2, 3, 4, 5, 6]);
        // Size 7 -> ArrayList after the next removal (swap first, from the set).
        assert!(w.remove(0));
        assert_eq!(w.get(), &[6, 7, 2, 3, 4, 5]);
        // ArrayList removal shifts.
        assert!(w.remove(7));
        assert_eq!(w.get(), &[6, 2, 3, 4, 5]);
    }
}
