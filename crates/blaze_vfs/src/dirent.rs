//! Port of `Dirent`.

use std::cmp::Ordering;
use std::fmt;

/// Type of a directory entry (Java `Dirent.Type`).
// Java: com/google/devtools/build/lib/vfs/Dirent.java:22 Dirent.Type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DirentType {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// A symlink.
    Symlink,
    /// None of the above. For example, a special file, or a path that could not be resolved while
    /// following symlinks.
    Unknown,
}

impl DirentType {
    fn lower_name(self) -> &'static str {
        match self {
            DirentType::File => "file",
            DirentType::Directory => "directory",
            DirentType::Symlink => "symlink",
            DirentType::Unknown => "unknown",
        }
    }
}

/// A directory entry: a name and a type (Java `Dirent`).
// Java: com/google/devtools/build/lib/vfs/Dirent.java:20 Dirent
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Dirent {
    name: String,
    dirent_type: DirentType,
}

impl Dirent {
    /// Creates a new dirent with the given name and type.
    // Java: com/google/devtools/build/lib/vfs/Dirent.java:38 Dirent.<init>
    pub fn new(name: impl Into<String>, dirent_type: DirentType) -> Dirent {
        Dirent {
            name: name.into(),
            dirent_type,
        }
    }

    /// The entry name.
    // Java: com/google/devtools/build/lib/vfs/Dirent.java:43 Dirent.getName
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// The entry type.
    // Java: com/google/devtools/build/lib/vfs/Dirent.java:47 Dirent.getType
    pub fn get_type(&self) -> DirentType {
        self.dirent_type
    }
}

// Java: com/google/devtools/build/lib/vfs/Dirent.java:68 Dirent.toString
impl fmt::Display for Dirent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}[{}]", self.name, self.dirent_type.lower_name())
    }
}

/// Orders by name (Java `Dirent.compareTo`). Java compares names only, which is inconsistent with
/// `equals`; Rust breaks name ties by type so that `Ord` agrees with `Eq`.
// Java: com/google/devtools/build/lib/vfs/Dirent.java:73 Dirent.compareTo
impl Ord for Dirent {
    fn cmp(&self, other: &Self) -> Ordering {
        self.name
            .as_bytes()
            .cmp(other.name.as_bytes())
            .then(self.dirent_type.cmp(&other.dirent_type))
    }
}

impl PartialOrd for Dirent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
