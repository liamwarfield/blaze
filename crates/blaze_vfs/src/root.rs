//! Port of `Root`: a root used by [`crate::rooted_path::RootedPath`] and artifact roots.
//!
//! Not ported: `RootCodec` / `RootCodecDependencies` (serialization).

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use crate::file_system::{FileSystemExt, FileSystemRef, fs_addr};
use crate::path::Path;
use crate::path_fragment::PathFragment;

/// A root: either a directory [`Path`] (Java `Root.PathRoot`) or the absolute root of a file
/// system (Java `Root.AbsoluteRoot`), which only resolves absolute fragments.
// Java: com/google/devtools/build/lib/vfs/Root.java:34 Root
#[derive(Clone)]
pub struct Root(RootKind);

#[derive(Clone)]
enum RootKind {
    // Java: com/google/devtools/build/lib/vfs/Root.java:84 Root.PathRoot
    PathRoot(Path),
    // Java: com/google/devtools/build/lib/vfs/Root.java:173 Root.AbsoluteRoot
    AbsoluteRoot(FileSystemRef),
}

impl Root {
    /// Constructs a root from a path.
    // Java: com/google/devtools/build/lib/vfs/Root.java:37 Root.fromPath
    pub fn from_path(path: Path) -> Root {
        Root(RootKind::PathRoot(path))
    }

    /// Returns the absolute root of `file_system`.
    // Java: com/google/devtools/build/lib/vfs/Root.java:42 Root.absoluteRoot
    pub fn absolute_root(file_system: &FileSystemRef) -> Root {
        Root(RootKind::AbsoluteRoot(file_system.clone()))
    }

    /// The same root on another file system.
    // Java: com/google/devtools/build/lib/vfs/Root.java:46 Root.toFileSystem
    pub fn to_file_system(root: &Root, file_system: &FileSystemRef) -> Root {
        match &root.0 {
            RootKind::AbsoluteRoot(_) => Root::absolute_root(file_system),
            RootKind::PathRoot(path) => {
                Root::from_path(file_system.get_path_fragment(path.as_fragment()))
            }
        }
    }

    /// Concatenates the root and a root-relative path. For the absolute root the fragment must be
    /// absolute (panics otherwise).
    // Java: com/google/devtools/build/lib/vfs/Root.java:53 Root.getRelative
    pub fn get_relative(&self, root_relative_path: &PathFragment) -> Path {
        match &self.0 {
            // Java: com/google/devtools/build/lib/vfs/Root.java:92 Root.PathRoot.getRelative
            RootKind::PathRoot(path) => path.get_relative(root_relative_path),
            // Java: com/google/devtools/build/lib/vfs/Root.java:181 Root.AbsoluteRoot.getRelative
            RootKind::AbsoluteRoot(fs) => {
                assert!(root_relative_path.is_absolute());
                fs.get_path_fragment(root_relative_path)
            }
        }
    }

    /// Concatenates the root and a root-relative path string.
    // Java: com/google/devtools/build/lib/vfs/Root.java:56 Root.getRelative
    pub fn get_relative_str(&self, root_relative_path: &str) -> Path {
        match &self.0 {
            // Java: com/google/devtools/build/lib/vfs/Root.java:97 Root.PathRoot.getRelative
            RootKind::PathRoot(path) => path.get_relative_str(root_relative_path),
            // Java: com/google/devtools/build/lib/vfs/Root.java:187 Root.AbsoluteRoot.getRelative
            RootKind::AbsoluteRoot(_) => {
                self.get_relative(&PathFragment::create(root_relative_path))
            }
        }
    }

    /// The path relative to this root. Panics if `path` is not under it.
    // Java: com/google/devtools/build/lib/vfs/Root.java:59 Root.relativize
    pub fn relativize(&self, path: &Path) -> PathFragment {
        match &self.0 {
            // Java: com/google/devtools/build/lib/vfs/Root.java:102 Root.PathRoot.relativize
            RootKind::PathRoot(root) => path.relative_to(root),
            // Java: com/google/devtools/build/lib/vfs/Root.java:192 Root.AbsoluteRoot.relativize
            RootKind::AbsoluteRoot(_) => path.as_fragment().clone(),
        }
    }

    /// The absolute fragment relative to this root. Panics if the fragment is relative or not
    /// under the root.
    // Java: com/google/devtools/build/lib/vfs/Root.java:62 Root.relativize
    pub fn relativize_fragment(&self, absolute_path_fragment: &PathFragment) -> PathFragment {
        assert!(absolute_path_fragment.is_absolute());
        match &self.0 {
            // Java: com/google/devtools/build/lib/vfs/Root.java:107 Root.PathRoot.relativize
            RootKind::PathRoot(root) => absolute_path_fragment.relative_to(root.as_fragment()),
            // Java: com/google/devtools/build/lib/vfs/Root.java:197 Root.AbsoluteRoot.relativize
            RootKind::AbsoluteRoot(_) => absolute_path_fragment.clone(),
        }
    }

    /// Whether `path` is under this root.
    // Java: com/google/devtools/build/lib/vfs/Root.java:65 Root.contains
    pub fn contains(&self, path: &Path) -> bool {
        match &self.0 {
            // Java: com/google/devtools/build/lib/vfs/Root.java:113 Root.PathRoot.contains
            RootKind::PathRoot(root) => path.starts_with(root),
            // Java: com/google/devtools/build/lib/vfs/Root.java:203 Root.AbsoluteRoot.contains
            RootKind::AbsoluteRoot(_) => true,
        }
    }

    /// Whether the absolute fragment is under this root.
    // Java: com/google/devtools/build/lib/vfs/Root.java:68 Root.contains
    pub fn contains_fragment(&self, absolute_path_fragment: &PathFragment) -> bool {
        match &self.0 {
            // Java: com/google/devtools/build/lib/vfs/Root.java:118 Root.PathRoot.contains
            RootKind::PathRoot(root) => {
                absolute_path_fragment.is_absolute()
                    && absolute_path_fragment.starts_with(root.as_fragment())
            }
            // Java: com/google/devtools/build/lib/vfs/Root.java:208 Root.AbsoluteRoot.contains
            RootKind::AbsoluteRoot(_) => absolute_path_fragment.is_absolute(),
        }
    }

    /// The underlying path; `None` for the absolute root.
    // Java: com/google/devtools/build/lib/vfs/Root.java:76 Root.asPath
    pub fn as_path(&self) -> Option<&Path> {
        match &self.0 {
            RootKind::PathRoot(path) => Some(path),
            RootKind::AbsoluteRoot(_) => None,
        }
    }

    /// The file system this root is on.
    // Java: com/google/devtools/build/lib/vfs/Root.java:79 Root.getFileSystem
    pub fn get_file_system(&self) -> &FileSystemRef {
        match &self.0 {
            RootKind::PathRoot(path) => path.get_file_system(),
            RootKind::AbsoluteRoot(fs) => fs,
        }
    }

    /// Whether this is an absolute root.
    // Java: com/google/devtools/build/lib/vfs/Root.java:81 Root.isAbsolute
    pub fn is_absolute(&self) -> bool {
        matches!(self.0, RootKind::AbsoluteRoot(_))
    }
}

// Java: com/google/devtools/build/lib/vfs/Root.java:139 Root.PathRoot.toString
// Java: com/google/devtools/build/lib/vfs/Root.java:229 Root.AbsoluteRoot.toString
impl fmt::Display for Root {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            RootKind::PathRoot(path) => fmt::Display::fmt(path, f),
            RootKind::AbsoluteRoot(_) => f.write_str("<absolute root>"),
        }
    }
}

impl fmt::Debug for Root {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Root({})", self)
    }
}

// Java: com/google/devtools/build/lib/vfs/Root.java:155 Root.PathRoot.equals
// Java: com/google/devtools/build/lib/vfs/Root.java:245 Root.AbsoluteRoot.equals
impl PartialEq for Root {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (RootKind::PathRoot(a), RootKind::PathRoot(b)) => a == b,
            (RootKind::AbsoluteRoot(a), RootKind::AbsoluteRoot(b)) => fs_addr(a) == fs_addr(b),
            _ => false,
        }
    }
}

impl Eq for Root {}

// Java: com/google/devtools/build/lib/vfs/Root.java:167 Root.PathRoot.hashCode
// Java: com/google/devtools/build/lib/vfs/Root.java:256 Root.AbsoluteRoot.hashCode
impl Hash for Root {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match &self.0 {
            RootKind::PathRoot(path) => path.hash(state),
            RootKind::AbsoluteRoot(fs) => (31usize.wrapping_add(fs_addr(fs))).hash(state),
        }
    }
}

/// Absolute roots sort before path roots; absolute roots compare by file system identity (Java
/// compares identity hash codes); path roots compare by path.
// Java: com/google/devtools/build/lib/vfs/Root.java:144 Root.PathRoot.compareTo
// Java: com/google/devtools/build/lib/vfs/Root.java:234 Root.AbsoluteRoot.compareTo
impl Ord for Root {
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.0, &other.0) {
            (RootKind::PathRoot(a), RootKind::PathRoot(b)) => a.cmp(b),
            (RootKind::PathRoot(_), RootKind::AbsoluteRoot(_)) => Ordering::Greater,
            (RootKind::AbsoluteRoot(_), RootKind::PathRoot(_)) => Ordering::Less,
            (RootKind::AbsoluteRoot(a), RootKind::AbsoluteRoot(b)) => fs_addr(a).cmp(&fs_addr(b)),
        }
    }
}

impl PartialOrd for Root {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
