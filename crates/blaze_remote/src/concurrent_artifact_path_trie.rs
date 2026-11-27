//! Port of `lib/remote/ConcurrentArtifactPathTrie.java`.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use blaze_exec::actions::ActionInputRef;
use blaze_vfs::PathFragment;
use parking_lot::Mutex;

/// A path ordered by `PathFragment.HIERARCHICAL_COMPARATOR`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Hierarchical(PathFragment);

impl Ord for Hierarchical {
    fn cmp(&self, other: &Self) -> Ordering {
        PathFragment::hierarchical_compare(&self.0, &other.0)
    }
}

impl PartialOrd for Hierarchical {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A set of artifact exec paths that answers whether a path is contained in (is, or is below)
/// one of them.
///
/// PORT: Java's `ConcurrentSkipListSet` is a locked `BTreeSet` here.
// Java: com/google/devtools/build/lib/remote/ConcurrentArtifactPathTrie.java:31 ConcurrentArtifactPathTrie
#[derive(Debug, Default)]
pub(crate) struct ConcurrentArtifactPathTrie {
    // Invariant: no path in this set is a prefix of another path.
    paths: Mutex<BTreeSet<Hierarchical>>,
}

impl ConcurrentArtifactPathTrie {
    /// Adds the given `ActionInput` to the trie.
    ///
    /// The caller must ensure that no object's path passed to this method is a prefix of any
    /// previously added object's path. Bazel enforces this for non-aggregate artifacts. Callers
    /// must not pass in `TreeFileArtifact`s (which have exec paths that have their parent tree
    /// artifact's exec path as a prefix) or non-Artifact `ActionInput`s that violate this
    /// invariant.
    // Java: com/google/devtools/build/lib/remote/ConcurrentArtifactPathTrie.java:46 ConcurrentArtifactPathTrie.add
    pub(crate) fn add(&self, input: &ActionInputRef) {
        blaze_util::check_argument!(
            !blaze_exec::actions::action_adapters::as_blaze_artifact(input).is_some_and(|a| {
                a.artifact_class()
                    == blaze_actions::actions::artifact::ArtifactClass::TreeFileArtifact
            }),
            "TreeFileArtifacts should not be added to the trie: {:?}",
            input
        );
        self.paths
            .lock()
            .insert(Hierarchical(input.0.get_exec_path().clone()));
    }

    /// Checks whether the given `PathFragment` is contained in an artifact in the trie.
    // Java: com/google/devtools/build/lib/remote/ConcurrentArtifactPathTrie.java:55 ConcurrentArtifactPathTrie.contains
    pub(crate) fn contains(&self, exec_path: &PathFragment) -> bool {
        // By the invariant of this set, there is at most one prefix of execPath in the set. Since
        // the comparator sorts all children of a path right after the path itself, if such a
        // prefix exists, it must thus sort right before execPath (or be equal to it).
        let paths = self.paths.lock();
        let floor_path = paths
            .range(..=Hierarchical(exec_path.clone()))
            .next_back();
        floor_path.is_some_and(|floor_path| exec_path.starts_with(&floor_path.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(path: &str) -> ActionInputRef {
        ActionInputRef::new(blaze_exec::actions::action_input::BasicActionInput::from_path(path))
    }

    #[test]
    fn contains_paths_below_an_added_path() {
        let trie = ConcurrentArtifactPathTrie::default();
        trie.add(&input("bazel-out/k8/bin/a/tree"));
        trie.add(&input("bazel-out/k8/bin/b"));
        assert!(trie.contains(&PathFragment::create("bazel-out/k8/bin/a/tree")));
        assert!(trie.contains(&PathFragment::create("bazel-out/k8/bin/a/tree/x/y")));
        assert!(trie.contains(&PathFragment::create("bazel-out/k8/bin/b")));
        assert!(!trie.contains(&PathFragment::create("bazel-out/k8/bin/a/tree2")));
        assert!(!trie.contains(&PathFragment::create("bazel-out/k8/bin/a")));
        assert!(!trie.contains(&PathFragment::create("bazel-out/k8/bin/c")));
    }
}
