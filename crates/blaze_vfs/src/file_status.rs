//! Port of `FileStatus` (as implemented by `UnixFileStatus`).

use crate::dirent::DirentType;

const S_IFMT: u32 = 0o170000;
const S_IFSOCK: u32 = 0o140000;
const S_IFLNK: u32 = 0o120000;
const S_IFREG: u32 = 0o100000;
const S_IFBLK: u32 = 0o060000;
const S_IFDIR: u32 = 0o040000;
const S_IFCHR: u32 = 0o020000;
const S_IFIFO: u32 = 0o010000;
// Java: com/google/devtools/build/lib/unix/UnixFileStatus.java:156 UnixFileStatus.S_IRWXA
const S_IRWXA: u32 = 0o0777;

/// The metadata a stat carries: Java's `FileStatusWithMetadata.getMetadata()`, a
/// `FileArtifactValue` (`blaze_exec`'s type, which this crate cannot name; `blaze_exec`
/// implements this trait for it).
///
/// PORT: Java's stat object is itself the `FileStatusWithMetadata`; here a [`FileStatus`] is a
/// plain value and the metadata is asked for separately
/// (`crate::FileSystem::stat_metadata`).
// Java: com/google/devtools/build/lib/actions/FileStatusWithMetadata.java:25 FileStatusWithMetadata
pub trait StatMetadata: std::any::Any + Send + Sync + std::fmt::Debug {
    /// `getMetadata().getSize()`.
    fn get_size(&self) -> i64;
    /// `getMetadata().getDigest()`.
    fn get_digest(&self) -> Option<Vec<u8>>;
    /// `getMetadata().getContentsProxy()` as (ctime, mtime, nodeId), `None` when it has none.
    fn get_contents_proxy(&self) -> Option<(i64, i64, i64)>;
    /// `getMetadata().equals(other)`.
    fn eq_dyn(&self, other: &dyn StatMetadata) -> bool;
    /// `getMetadata().hashCode()`, into `state`.
    fn hash_dyn(&self, state: &mut dyn std::hash::Hasher);
    /// The metadata itself, for a downcast to its type.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// The status of a file (Java `FileStatus`, data as in `UnixFileStatus`).
// Java: com/google/devtools/build/lib/vfs/FileStatus.java:37 FileStatus
// Java: com/google/devtools/build/lib/unix/UnixFileStatus.java:20 UnixFileStatus
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileStatus {
    mode: u32,
    /// milliseconds since Unix epoch
    mtime: i64,
    /// milliseconds since Unix epoch
    ctime: i64,
    size: i64,
    ino: i64,
}

impl FileStatus {
    /// Creates a status from raw Unix `stat` fields; times are milliseconds since the epoch.
    // Java: com/google/devtools/build/lib/unix/UnixFileStatus.java:28 UnixFileStatus.<init>
    pub fn new(mode: u32, mtime: i64, ctime: i64, size: i64, ino: i64) -> FileStatus {
        FileStatus {
            mode,
            mtime,
            ctime,
            size,
            ino,
        }
    }

    /// Creates a status from `std` metadata.
    pub fn from_metadata(m: &std::fs::Metadata) -> FileStatus {
        use std::os::unix::fs::MetadataExt;
        FileStatus::new(
            m.mode(),
            m.mtime() * 1000 + m.mtime_nsec() / 1_000_000,
            m.ctime() * 1000 + m.ctime_nsec() / 1_000_000,
            m.size() as i64,
            m.ino() as i64,
        )
    }

    /// Returns true iff this file is a regular file or a special file.
    // Java: com/google/devtools/build/lib/unix/UnixFileStatus.java:66 UnixFileStatus.isFile
    pub fn is_file(&self) -> bool {
        let t = self.mode & S_IFMT;
        t == S_IFREG || self.is_special_file()
    }

    /// Returns true iff this file is a directory.
    // Java: com/google/devtools/build/lib/unix/UnixFileStatus.java:86 UnixFileStatus.isDirectory
    pub fn is_directory(&self) -> bool {
        self.mode & S_IFMT == S_IFDIR
    }

    /// Returns true iff this file is a symbolic link.
    // Java: com/google/devtools/build/lib/unix/UnixFileStatus.java:96 UnixFileStatus.isSymbolicLink
    pub fn is_symbolic_link(&self) -> bool {
        self.mode & S_IFMT == S_IFLNK
    }

    /// Returns true iff this file is a special file (socket, fifo or device).
    // Java: com/google/devtools/build/lib/unix/UnixFileStatus.java:76 UnixFileStatus.isSpecialFile
    pub fn is_special_file(&self) -> bool {
        let t = self.mode & S_IFMT;
        t == S_IFSOCK || t == S_IFBLK || t == S_IFCHR || t == S_IFIFO
    }

    /// File size in bytes.
    // Java: com/google/devtools/build/lib/unix/UnixFileStatus.java:111 UnixFileStatus.getSize
    pub fn get_size(&self) -> i64 {
        self.size
    }

    /// Last modification time, milliseconds since the Unix epoch.
    // Java: com/google/devtools/build/lib/unix/UnixFileStatus.java:116 UnixFileStatus.getLastModifiedTime
    pub fn get_last_modified_time(&self) -> i64 {
        self.mtime
    }

    /// Last status change time, milliseconds since the Unix epoch.
    // Java: com/google/devtools/build/lib/unix/UnixFileStatus.java:121 UnixFileStatus.getLastChangeTime
    pub fn get_last_change_time(&self) -> i64 {
        self.ctime
    }

    /// The inode number.
    // Java: com/google/devtools/build/lib/unix/UnixFileStatus.java:60 UnixFileStatus.getNodeId
    pub fn get_node_id(&self) -> i64 {
        self.ino
    }

    /// Permission bits (`mode & 0777`).
    // Java: com/google/devtools/build/lib/unix/UnixFileStatus.java:106 UnixFileStatus.getPermissions
    pub fn get_permissions(&self) -> u32 {
        self.mode & S_IRWXA
    }

    /// The dirent type for this status.
    // Java: com/google/devtools/build/lib/unix/UnixFileStatus.java:55 UnixFileStatus.getDirentType
    pub fn get_dirent_type(&self) -> DirentType {
        if self.is_special_file() {
            DirentType::Unknown
        } else if self.is_file() {
            DirentType::File
        } else if self.is_directory() {
            DirentType::Directory
        } else if self.is_symbolic_link() {
            DirentType::Symlink
        } else {
            DirentType::Unknown
        }
    }
}
