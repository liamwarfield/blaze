//! Port of `Symlinks`.

/// Whether to follow symbolic links (Java `Symlinks`).
// Java: com/google/devtools/build/lib/vfs/Symlinks.java:20 Symlinks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Symlinks {
    /// Follow symbolic links.
    Follow,
    /// Do not follow symbolic links.
    Nofollow,
}

impl Symlinks {
    /// Returns true for [`Symlinks::Follow`].
    // Java: com/google/devtools/build/lib/vfs/Symlinks.java:28 Symlinks.toBoolean
    pub fn to_boolean(self) -> bool {
        self == Symlinks::Follow
    }
}
