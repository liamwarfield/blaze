//! Port of `collect/PathFragmentPrefixTrie.java`: a trie of path prefixes, each included or
//! excluded, answering whether a path is under an included prefix (the project directories of a
//! `PROJECT.scl`).
//!
//! PORT: Java's class is in `lib.collect`; `blaze_collect` does not depend on `blaze_vfs`
//! (PathFragment), so it lives here next to its users (project files).

use std::collections::{BTreeSet, HashMap};

use blaze_vfs::PathFragment;
use indexmap::IndexMap;

/// The kind of a trie segment.
// Java: com/google/devtools/build/lib/collect/PathFragmentPrefixTrie.java:59 PathFragmentPrefixTrie.Segment
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SegmentKind {
    Interim,
    Excluded,
    Included,
}

#[derive(Debug)]
struct Segment {
    kind: SegmentKind,
    segment_map: HashMap<String, Segment>,
}

impl Segment {
    fn new(kind: SegmentKind) -> Segment {
        Segment {
            kind,
            segment_map: HashMap::new(),
        }
    }
}

// Java: com/google/devtools/build/lib/collect/PathFragmentPrefixTrie.java:48 PathFragmentPrefixTrie
/// A prefix trie of included and excluded path fragments.
#[derive(Debug)]
pub struct PathFragmentPrefixTrie {
    included_paths: BTreeSet<PathFragment>,
    excluded_paths: BTreeSet<PathFragment>,
    root: Segment,
}

/// Java `PathFragmentPrefixTrieException` (only `PathFragmentAlreadyAddedException` is thrown).
// Java: com/google/devtools/build/lib/collect/PathFragmentPrefixTrie.java:194 PathFragmentPrefixTrie.PathFragmentPrefixTrieException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct PathFragmentPrefixTrieException(pub String);

impl Default for PathFragmentPrefixTrie {
    fn default() -> Self {
        Self::new()
    }
}

impl PathFragmentPrefixTrie {
    // Java: com/google/devtools/build/lib/collect/PathFragmentPrefixTrie.java:97 PathFragmentPrefixTrie.<init>
    pub fn new() -> PathFragmentPrefixTrie {
        PathFragmentPrefixTrie {
            included_paths: BTreeSet::new(),
            excluded_paths: BTreeSet::new(),
            root: Segment::new(SegmentKind::Interim),
        }
    }

    // Java: com/google/devtools/build/lib/collect/PathFragmentPrefixTrie.java:106 PathFragmentPrefixTrie.of
    /// Builds a trie from paths; a path starting with `-` is an exclusion.
    pub fn of<'a>(
        paths: impl IntoIterator<Item = &'a String>,
    ) -> Result<PathFragmentPrefixTrie, PathFragmentPrefixTrieException> {
        let mut trie = PathFragmentPrefixTrie::new();
        for p in paths {
            if let Some(excluded) = p.strip_prefix('-') {
                // Exclusion
                trie.put(&PathFragment::create(excluded), false)?;
            } else {
                // Inclusion
                trie.put(&PathFragment::create(p), true)?;
            }
        }
        Ok(trie)
    }

    // Java: com/google/devtools/build/lib/collect/PathFragmentPrefixTrie.java:122 PathFragmentPrefixTrie.transformValues
    pub fn transform_values(
        map: &IndexMap<String, Vec<String>>,
    ) -> Result<IndexMap<String, PathFragmentPrefixTrie>, PathFragmentPrefixTrieException> {
        let mut builder = IndexMap::new();
        for (key, value) in map {
            builder.insert(key.clone(), PathFragmentPrefixTrie::of(value)?);
        }
        Ok(builder)
    }

    // Java: com/google/devtools/build/lib/collect/PathFragmentPrefixTrie.java:139 PathFragmentPrefixTrie.put
    pub fn put(
        &mut self,
        path_fragment: &PathFragment,
        included: bool,
    ) -> Result<(), PathFragmentPrefixTrieException> {
        blaze_util::check_argument!(
            *path_fragment != PathFragment::empty_fragment(),
            "path fragment cannot be the empty fragment."
        );
        let trie_string = self.to_string();
        let segments: Vec<String> = path_fragment.segments().map(|s| s.to_string()).collect();
        let mut current = &mut self.root;
        for (i, next_segment) in segments.iter().enumerate() {
            if i + 1 < segments.len() {
                current = current
                    .segment_map
                    .entry(next_segment.clone())
                    .or_insert_with(|| Segment::new(SegmentKind::Interim));
                continue;
            }
            // This is the last segment.
            let new_kind = if included {
                SegmentKind::Included
            } else {
                SegmentKind::Excluded
            };
            match current.segment_map.get_mut(next_segment) {
                Some(segment) => match segment.kind {
                    SegmentKind::Interim => segment.kind = new_kind,
                    SegmentKind::Excluded | SegmentKind::Included => {
                        // Java: com/google/devtools/build/lib/collect/PathFragmentPrefixTrie.java:205 PathFragmentPrefixTrie.PathFragmentAlreadyAddedException.<init>
                        return Err(PathFragmentPrefixTrieException(format!(
                            "{} has already been explicitly marked as {}. Current state: {}",
                            path_fragment,
                            if segment.kind == SegmentKind::Included {
                                "included"
                            } else {
                                "excluded"
                            },
                            trie_string
                        )));
                    }
                },
                None => {
                    current
                        .segment_map
                        .insert(next_segment.clone(), Segment::new(new_kind));
                }
            }
        }
        if included {
            self.included_paths.insert(path_fragment.clone());
        } else {
            self.excluded_paths.insert(path_fragment.clone());
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/collect/PathFragmentPrefixTrie.java:171 PathFragmentPrefixTrie.includes
    pub fn includes(&self, path_fragment: &PathFragment) -> bool {
        if *path_fragment == PathFragment::empty_fragment() {
            return false;
        }
        let mut current = &self.root;
        let mut last_kind = current.kind;
        for next_segment in path_fragment.segments() {
            match current.segment_map.get(next_segment) {
                None => break,
                Some(next) => {
                    current = next;
                    if current.kind != SegmentKind::Interim {
                        last_kind = current.kind; // either Included or Excluded
                    }
                }
            }
        }
        last_kind == SegmentKind::Included
    }

    // Java: com/google/devtools/build/lib/collect/PathFragmentPrefixTrie.java:218 PathFragmentPrefixTrie.hasIncludedPaths
    pub fn has_included_paths(&self) -> bool {
        !self.included_paths.is_empty()
    }
}

impl std::fmt::Display for PathFragmentPrefixTrie {
    // Java: com/google/devtools/build/lib/collect/PathFragmentPrefixTrie.java:186 PathFragmentPrefixTrie.toString
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let list = |s: &BTreeSet<PathFragment>| {
            s.iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        write!(
            f,
            "[included: [{}], excluded: [{}]]",
            list(&self.included_paths),
            list(&self.excluded_paths)
        )
    }
}
