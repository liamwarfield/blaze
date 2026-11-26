//! `ModifiedFileSet`: an immutable set of modified source files.

use std::fmt;

use indexmap::IndexSet;

use crate::PathFragment;

/// An immutable set of modified source files. The scope of these files is context-dependent; in
/// some uses this may mean information about all files in the client, while in other uses this
/// may mean information about some specific subset of files.
/// [`ModifiedFileSet::EVERYTHING_MODIFIED`] can be used to indicate that all files of interest
/// have been modified.
///
/// PORT: Java's anonymous subclass `EVERYTHING_DELETED` is the `everything_deleted` flag; the
/// Skycache hooks (`getInstructionsMessage`, `getInstructionsPrelude`, `getMessages`, all null or
/// empty in Bazel) are not ported.
// Java: com/google/devtools/build/lib/vfs/ModifiedFileSet.java:28 ModifiedFileSet
#[derive(Clone, PartialEq, Eq)]
pub struct ModifiedFileSet {
    /// `None`: everything is modified.
    modified: Option<IndexSet<PathFragment>>,
    everything_deleted: bool,
}

impl ModifiedFileSet {
    // When everything is modified that naturally includes all directories.
    // Java: com/google/devtools/build/lib/vfs/ModifiedFileSet.java:54 ModifiedFileSet.EVERYTHING_MODIFIED
    pub const EVERYTHING_MODIFIED: ModifiedFileSet = ModifiedFileSet {
        modified: None,
        everything_deleted: false,
    };

    /// Special case of [`Self::EVERYTHING_MODIFIED`], which indicates that the entire tree has
    /// been deleted.
    // Java: com/google/devtools/build/lib/vfs/ModifiedFileSet.java:60 ModifiedFileSet.EVERYTHING_DELETED
    pub const EVERYTHING_DELETED: ModifiedFileSet = ModifiedFileSet {
        modified: None,
        everything_deleted: true,
    };

    // Java: com/google/devtools/build/lib/vfs/ModifiedFileSet.java:68 ModifiedFileSet.NOTHING_MODIFIED
    pub fn nothing_modified() -> ModifiedFileSet {
        ModifiedFileSet {
            modified: Some(IndexSet::new()),
            everything_deleted: false,
        }
    }

    /// Whether all files of interest should be treated as potentially modified.
    // Java: com/google/devtools/build/lib/vfs/ModifiedFileSet.java:75 ModifiedFileSet.treatEverythingAsModified
    pub fn treat_everything_as_modified(&self) -> bool {
        self.modified.is_none()
    }

    /// Returns whether the diff indicates the whole tree has been deleted.
    ///
    /// This precludes any optimizations like skipping invalidation when we do not check modified
    /// outputs.
    // Java: com/google/devtools/build/lib/vfs/ModifiedFileSet.java:85 ModifiedFileSet.treatEverythingAsDeleted
    pub fn treat_everything_as_deleted(&self) -> bool {
        self.everything_deleted
    }

    /// The set of files of interest that were modified.
    ///
    /// Panics (Java: `IllegalStateException`) if [`Self::treat_everything_as_modified`] returns
    /// true.
    // Java: com/google/devtools/build/lib/vfs/ModifiedFileSet.java:94 ModifiedFileSet.modifiedSourceFiles
    pub fn modified_source_files(&self) -> &IndexSet<PathFragment> {
        match &self.modified {
            Some(modified) => modified,
            None => panic!("IllegalStateException"),
        }
    }

    /// Whether this is [`Self::nothing_modified`] (Java compares with the constant by `equals`).
    pub fn is_nothing_modified(&self) -> bool {
        *self == Self::nothing_modified()
    }

    // Java: com/google/devtools/build/lib/vfs/ModifiedFileSet.java:150 ModifiedFileSet.builder
    pub fn builder() -> ModifiedFileSetBuilder {
        ModifiedFileSetBuilder::default()
    }
}

impl fmt::Display for ModifiedFileSet {
    // Java: com/google/devtools/build/lib/vfs/ModifiedFileSet.java:118 ModifiedFileSet.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == Self::EVERYTHING_DELETED {
            f.write_str("EVERYTHING_DELETED")
        } else if *self == Self::EVERYTHING_MODIFIED {
            f.write_str("EVERYTHING_MODIFIED")
        } else if self.is_nothing_modified() {
            f.write_str("NOTHING_MODIFIED")
        } else {
            // ImmutableSet.toString: "[a, b]".
            let items: Vec<&str> = self
                .modified_source_files()
                .iter()
                .map(|p| p.get_path_string())
                .collect();
            write!(f, "[{}]", items.join(", "))
        }
    }
}

impl fmt::Debug for ModifiedFileSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// The builder for [`ModifiedFileSet`].
// Java: com/google/devtools/build/lib/vfs/ModifiedFileSet.java:133 ModifiedFileSet.Builder
#[derive(Default)]
pub struct ModifiedFileSetBuilder {
    set_builder: IndexSet<PathFragment>,
}

impl ModifiedFileSetBuilder {
    // Java: com/google/devtools/build/lib/vfs/ModifiedFileSet.java:136 ModifiedFileSet.Builder.build
    pub fn build(self) -> ModifiedFileSet {
        if self.set_builder.is_empty() {
            ModifiedFileSet::nothing_modified()
        } else {
            ModifiedFileSet {
                modified: Some(self.set_builder),
                everything_deleted: false,
            }
        }
    }

    // Java: com/google/devtools/build/lib/vfs/ModifiedFileSet.java:141 ModifiedFileSet.Builder.modify
    pub fn modify(mut self, path_fragment: PathFragment) -> Self {
        self.set_builder.insert(path_fragment);
        self
    }

    // Java: com/google/devtools/build/lib/vfs/ModifiedFileSet.java:147 ModifiedFileSet.Builder.modifyAll
    pub fn modify_all(mut self, path_fragments: impl IntoIterator<Item = PathFragment>) -> Self {
        self.set_builder.extend(path_fragments);
        self
    }
}

#[cfg(test)]
mod tests {
    // Bazel has no JUnit class for ModifiedFileSet; these follow its methods.
    use super::*;

    #[test]
    fn constants_and_builder() {
        assert!(ModifiedFileSet::EVERYTHING_MODIFIED.treat_everything_as_modified());
        assert!(!ModifiedFileSet::EVERYTHING_MODIFIED.treat_everything_as_deleted());
        assert!(ModifiedFileSet::EVERYTHING_DELETED.treat_everything_as_modified());
        assert!(ModifiedFileSet::EVERYTHING_DELETED.treat_everything_as_deleted());
        assert_ne!(
            ModifiedFileSet::EVERYTHING_MODIFIED,
            ModifiedFileSet::EVERYTHING_DELETED
        );
        assert_eq!(
            ModifiedFileSet::builder().build(),
            ModifiedFileSet::nothing_modified()
        );
        let set = ModifiedFileSet::builder()
            .modify(PathFragment::create("a/b"))
            .modify_all([PathFragment::create("c")])
            .build();
        assert!(!set.treat_everything_as_modified());
        assert_eq!(set.modified_source_files().len(), 2);
        assert_eq!(set.to_string(), "[a/b, c]");
        assert_eq!(
            ModifiedFileSet::EVERYTHING_MODIFIED.to_string(),
            "EVERYTHING_MODIFIED"
        );
        assert_eq!(
            ModifiedFileSet::EVERYTHING_DELETED.to_string(),
            "EVERYTHING_DELETED"
        );
        assert_eq!(
            ModifiedFileSet::nothing_modified().to_string(),
            "NOTHING_MODIFIED"
        );
    }
}
