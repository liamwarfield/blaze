//! Port of `RootedPath`: a [`PathFragment`] relative to a [`Root`].
//!
//! Not ported: interning (`SkyKeyInterner`), `SkyKey`/`FileStateKey`, AutoCodec.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use crate::file_system::FileSystemExt;
use crate::path::Path;
use crate::path_fragment::PathFragment;
use crate::root::Root;

/// A path fragment relative to a root, typically a package path entry (Java `RootedPath`).
/// Two rooted paths are equal iff they have equal roots and equal relative paths.
// Java: com/google/devtools/build/lib/vfs/RootedPath.java:37 RootedPath
#[derive(Clone)]
pub struct RootedPath {
    root: Root,
    root_relative_path: PathFragment,
}

impl RootedPath {
    // Java: com/google/devtools/build/lib/vfs/RootedPath.java:51 RootedPath.createInternal
    fn create_internal(root: Root, root_relative_path: PathFragment) -> RootedPath {
        assert!(
            root_relative_path.is_absolute() == root.is_absolute(),
            "rootRelativePath: {} root: {}",
            root_relative_path,
            root
        );
        RootedPath {
            root,
            root_relative_path,
        }
    }

    /// A rooted path for `root_relative_path` relative to `root`. An absolute fragment under a
    /// path root is relativized; panics if it is not under the root.
    // Java: com/google/devtools/build/lib/vfs/RootedPath.java:68 RootedPath.toRootedPath
    pub fn to_rooted_path(root: Root, root_relative_path: &PathFragment) -> RootedPath {
        let mut root_relative_path = root_relative_path.clone();
        if root_relative_path.is_absolute() && !root.is_absolute() {
            assert!(
                root.contains_fragment(&root_relative_path),
                "rootRelativePath '{}' is absolute, but it's not under root '{}'",
                root_relative_path,
                root
            );
            root_relative_path = root.relativize_fragment(&root_relative_path);
        }
        Self::create_internal(root, root_relative_path)
    }

    /// A rooted path for `path` under `root`. Panics if `path` is not under `root`.
    // Java: com/google/devtools/build/lib/vfs/RootedPath.java:81 RootedPath.toRootedPath
    pub fn to_rooted_path_from_path(root: Root, path: &Path) -> RootedPath {
        assert!(root.contains(path), "path: {} root: {}", path, root);
        Self::to_rooted_path(root, path.as_fragment())
    }

    /// A rooted path for `path` under the first containing root, or the absolute root.
    // Java: com/google/devtools/build/lib/vfs/RootedPath.java:90 RootedPath.toRootedPathMaybeUnderRoot
    pub fn to_rooted_path_maybe_under_root<'a>(
        path: &Path,
        package_path_roots: impl IntoIterator<Item = &'a Root>,
    ) -> RootedPath {
        for root in package_path_roots {
            if root.contains(path) {
                return Self::to_rooted_path_from_path(root.clone(), path);
            }
        }
        Self::to_rooted_path_from_path(path.get_file_system().absolute_root(), path)
    }

    /// The full path.
    // Java: com/google/devtools/build/lib/vfs/RootedPath.java:99 RootedPath.asPath
    pub fn as_path(&self) -> Path {
        self.root.get_relative(&self.root_relative_path)
    }

    /// The root.
    // Java: com/google/devtools/build/lib/vfs/RootedPath.java:103 RootedPath.getRoot
    pub fn get_root(&self) -> &Root {
        &self.root
    }

    /// The path relative to the root.
    // Java: com/google/devtools/build/lib/vfs/RootedPath.java:108 RootedPath.getRootRelativePath
    pub fn get_root_relative_path(&self) -> &PathFragment {
        &self.root_relative_path
    }

    /// The parent within the same root, or `None` at the root.
    // Java: com/google/devtools/build/lib/vfs/RootedPath.java:113 RootedPath.getParentDirectory
    pub fn get_parent_directory(&self) -> Option<RootedPath> {
        let parent = self.root_relative_path.get_parent_directory()?;
        Some(Self::create_internal(self.root.clone(), parent))
    }
}

// Java: com/google/devtools/build/lib/vfs/RootedPath.java:122 RootedPath.equals
impl PartialEq for RootedPath {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root && self.root_relative_path == other.root_relative_path
    }
}

impl Eq for RootedPath {}

// Java: com/google/devtools/build/lib/vfs/RootedPath.java:135 RootedPath.hashCode
impl Hash for RootedPath {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.root.hash(state);
        self.root_relative_path.hash(state);
    }
}

// Java: com/google/devtools/build/lib/vfs/RootedPath.java:140 RootedPath.toString
impl fmt::Display for RootedPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}]/[{}]", self.root, self.root_relative_path)
    }
}

impl fmt::Debug for RootedPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RootedPath({})", self)
    }
}

// Java: com/google/devtools/build/lib/vfs/RootedPath.java:145 RootedPath.compareTo
impl Ord for RootedPath {
    fn cmp(&self, other: &Self) -> Ordering {
        self.root
            .cmp(&other.root)
            .then_with(|| self.root_relative_path.cmp(&other.root_relative_path))
    }
}

impl PartialOrd for RootedPath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
