//! Port of `Path`: an absolute path on a particular [`FileSystem`].

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};

use crate::dirent::Dirent;
use crate::file_status::FileStatus;
use crate::file_system::{FileSystemRef, fs_addr};
use crate::path_fragment::PathFragment;
use crate::symlinks::Symlinks;

/// Passed to [`Path::set_last_modified_time`] to mean "now".
// Java: com/google/devtools/build/lib/vfs/Path.java:60 Path.NOW_SENTINEL_TIME
pub const NOW_SENTINEL_TIME: i64 = -1;

/// An absolute path on a file system (Java `Path`).
///
/// Equality is the file system's identity (Java reference equality) plus the fragment; hashing
/// uses the fragment only; ordering is by file system address, then fragment.
// Java: com/google/devtools/build/lib/vfs/Path.java:58 Path
#[derive(Clone)]
pub struct Path {
    path_fragment: PathFragment,
    file_system: FileSystemRef,
}

// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:60 FileSystemUtils.checkBaseName
fn check_base_name(base_name: &str) {
    if base_name.is_empty() {
        panic!("Child must not be empty string ('')");
    }
    if base_name == "." || base_name == ".." {
        panic!("baseName must not be '{}'", base_name);
    }
    if base_name.contains('/') {
        panic!("baseName must not contain a slash: '{}'", base_name);
    }
}

impl Path {
    /// Creates a path from a string (normalized, no IO).
    // Java: com/google/devtools/build/lib/vfs/Path.java:66 Path.create
    pub fn create_from_str(path: &str, file_system: &FileSystemRef) -> Path {
        Path::create(PathFragment::create(path), file_system)
    }

    /// Creates a path. Panics (Java `IllegalArgumentException`) if the fragment is relative.
    // Java: com/google/devtools/build/lib/vfs/Path.java:72 Path.create
    pub fn create(path_fragment: PathFragment, file_system: &FileSystemRef) -> Path {
        // Java: com/google/devtools/build/lib/vfs/Path.java:77 Path.<init>
        assert!(
            path_fragment.is_absolute(),
            "Paths must be absolute: '{}'",
            path_fragment
        );
        Path {
            path_fragment,
            file_system: file_system.clone(),
        }
    }

    /// The normalized path string.
    // Java: com/google/devtools/build/lib/vfs/Path.java:83 Path.getPathString
    pub fn get_path_string(&self) -> &str {
        self.path_fragment.get_path_string()
    }

    /// The string used by file type matchers.
    // Java: com/google/devtools/build/lib/vfs/Path.java:88 Path.filePathForFileTypeMatcher
    pub fn file_path_for_file_type_matcher(&self) -> &str {
        self.path_fragment.get_path_string()
    }

    /// The name of the leaf file or directory; "" for '/'.
    // Java: com/google/devtools/build/lib/vfs/Path.java:98 Path.getBaseName
    pub fn get_base_name(&self) -> &str {
        self.path_fragment.get_base_name()
    }

    /// The parent directory with `new_name` resolved against it; `None` for '/'.
    // Java: com/google/devtools/build/lib/vfs/Path.java:110 Path.replaceName
    pub fn replace_name(&self, new_name: &str) -> Option<Path> {
        self.get_parent_directory()
            .map(|parent| parent.get_relative_str(new_name))
    }

    /// Appends one segment. Panics if `child` is empty, `.`, `..` or contains '/'.
    // Java: com/google/devtools/build/lib/vfs/Path.java:116 Path.getChild
    pub fn get_child(&self, child: &str) -> Path {
        check_base_name(child);
        self.get_relative_str(child)
    }

    /// Resolves `other` against this path.
    // Java: com/google/devtools/build/lib/vfs/Path.java:125 Path.getRelative
    pub fn get_relative(&self, other: &PathFragment) -> Path {
        Path::create(self.path_fragment.get_relative(other), &self.file_system)
    }

    /// Resolves the string `other` against this path.
    // Java: com/google/devtools/build/lib/vfs/Path.java:134 Path.getRelative
    pub fn get_relative_str(&self, other: &str) -> Path {
        Path::create(
            self.path_fragment.get_relative_str(other),
            &self.file_system,
        )
    }

    /// The parent directory; `None` for '/'.
    // Java: com/google/devtools/build/lib/vfs/Path.java:145 Path.getParentDirectory
    pub fn get_parent_directory(&self) -> Option<Path> {
        self.path_fragment
            .get_parent_directory()
            .map(|p| Path::create(p, &self.file_system))
    }

    /// This path relative to `base`. Panics if not under `base` or on another file system.
    // Java: com/google/devtools/build/lib/vfs/Path.java:163 Path.relativeTo
    pub fn relative_to(&self, base: &Path) -> PathFragment {
        self.check_same_file_system(base);
        self.path_fragment.relative_to(&base.path_fragment)
    }

    /// Whether `other` is an ancestor of (or equal to) this path, on the same file system.
    // Java: com/google/devtools/build/lib/vfs/Path.java:174 Path.startsWith
    pub fn starts_with(&self, other: &Path) -> bool {
        if fs_addr(&self.file_system) != fs_addr(&other.file_system) {
            return false;
        }
        self.path_fragment.starts_with(&other.path_fragment)
    }

    /// Like `starts_with`, ignoring case.
    // Java: com/google/devtools/build/lib/vfs/Path.java:186 Path.startsWithIgnoringCase
    pub fn starts_with_ignoring_case(&self, other: &Path) -> bool {
        if fs_addr(&self.file_system) != fs_addr(&other.file_system) {
            return false;
        }
        self.path_fragment
            .starts_with_ignoring_case(&other.path_fragment)
    }

    /// Whether the fragment `other` is an ancestor of this path.
    // Java: com/google/devtools/build/lib/vfs/Path.java:200 Path.startsWith
    pub fn starts_with_fragment(&self, other: &PathFragment) -> bool {
        self.path_fragment.starts_with(other)
    }

    /// The file system of this path.
    // Java: com/google/devtools/build/lib/vfs/Path.java:204 Path.getFileSystem
    pub fn get_file_system(&self) -> &FileSystemRef {
        &self.file_system
    }

    /// The absolute fragment of this path.
    // Java: com/google/devtools/build/lib/vfs/Path.java:208 Path.asFragment
    pub fn as_fragment(&self) -> &PathFragment {
        &self.path_fragment
    }

    /// Returns the path on the host file system: this path, unless its file system is based on
    /// another one (an action file system), then the same fragment on that file system.
    // Java: com/google/devtools/build/lib/vfs/Path.java:262 Path.forHostFileSystem
    pub fn for_host_file_system(&self) -> Path {
        match self.file_system.get_host_file_system() {
            // `hostFs.equals(fileSystem)`: `None` is "this file system".
            None => self.clone(),
            Some(host_fs) => Path::create(self.path_fragment.clone(), &host_fs),
        }
    }

    /// Whether the file exists, following symlinks.
    // Java: com/google/devtools/build/lib/vfs/Path.java:271 Path.exists
    pub fn exists(&self) -> bool {
        self.file_system.exists(&self.path_fragment, true)
    }

    /// Whether the file exists.
    // Java: com/google/devtools/build/lib/vfs/Path.java:281 Path.exists
    pub fn exists_with(&self, follow_symlinks: Symlinks) -> bool {
        self.file_system
            .exists(&self.path_fragment, follow_symlinks.to_boolean())
    }

    /// The entries of this directory as child paths.
    // Java: com/google/devtools/build/lib/vfs/Path.java:292 Path.getDirectoryEntries
    pub fn get_directory_entries(&self) -> io::Result<Vec<Path>> {
        let entries = self
            .file_system
            .get_directory_entries(&self.path_fragment)?;
        Ok(entries.iter().map(|e| self.get_child(e)).collect())
    }

    /// Names and types of the entries of this directory (order not guaranteed).
    // Java: com/google/devtools/build/lib/vfs/Path.java:310 Path.readdir
    pub fn readdir(&self, follow_symlinks: Symlinks) -> io::Result<Vec<Dirent>> {
        self.file_system
            .readdir(&self.path_fragment, follow_symlinks.to_boolean())
    }

    /// Status, following symlinks.
    // Java: com/google/devtools/build/lib/vfs/Path.java:321 Path.stat
    pub fn stat(&self) -> io::Result<FileStatus> {
        self.file_system.stat(&self.path_fragment, true)
    }

    /// Status.
    // Java: com/google/devtools/build/lib/vfs/Path.java:334 Path.stat
    pub fn stat_with(&self, follow_symlinks: Symlinks) -> io::Result<FileStatus> {
        self.file_system
            .stat(&self.path_fragment, follow_symlinks.to_boolean())
    }

    /// Status or `None` on any error.
    // Java: com/google/devtools/build/lib/vfs/Path.java:354 Path.statNullable
    pub fn stat_nullable(&self, follow_symlinks: Symlinks) -> Option<FileStatus> {
        self.file_system
            .stat_nullable(&self.path_fragment, follow_symlinks.to_boolean())
    }

    /// The metadata the stat of this path carries (`FileStatusWithMetadata.getMetadata()`), if
    /// any (see `FileSystem::stat_metadata`).
    pub fn stat_metadata(
        &self,
        follow_symlinks: Symlinks,
    ) -> io::Result<Option<std::sync::Arc<dyn crate::file_status::StatMetadata>>> {
        self.file_system
            .stat_metadata(&self.path_fragment, follow_symlinks.to_boolean())
    }

    /// Status or `None` if not found.
    // Java: com/google/devtools/build/lib/vfs/Path.java:378 Path.statIfFound
    pub fn stat_if_found(&self, follow_symlinks: Symlinks) -> io::Result<Option<FileStatus>> {
        self.file_system
            .stat_if_found(&self.path_fragment, follow_symlinks.to_boolean())
    }

    /// Whether this is a directory, following symlinks.
    // Java: com/google/devtools/build/lib/vfs/Path.java:383 Path.isDirectory
    pub fn is_directory(&self) -> bool {
        self.file_system.is_directory(&self.path_fragment, true)
    }

    /// Whether this is a directory.
    // Java: com/google/devtools/build/lib/vfs/Path.java:393 Path.isDirectory
    pub fn is_directory_with(&self, follow_symlinks: Symlinks) -> bool {
        self.file_system
            .is_directory(&self.path_fragment, follow_symlinks.to_boolean())
    }

    /// Whether this is a regular or special file, following symlinks.
    // Java: com/google/devtools/build/lib/vfs/Path.java:403 Path.isFile
    pub fn is_file(&self) -> bool {
        self.file_system.is_file(&self.path_fragment, true)
    }

    /// Whether this is a regular or special file.
    // Java: com/google/devtools/build/lib/vfs/Path.java:416 Path.isFile
    pub fn is_file_with(&self, follow_symlinks: Symlinks) -> bool {
        self.file_system
            .is_file(&self.path_fragment, follow_symlinks.to_boolean())
    }

    /// Whether this is a special file, following symlinks.
    // Java: com/google/devtools/build/lib/vfs/Path.java:424 Path.isSpecialFile
    pub fn is_special_file(&self) -> bool {
        self.file_system.is_special_file(&self.path_fragment, true)
    }

    /// Whether this is a special file.
    // Java: com/google/devtools/build/lib/vfs/Path.java:434 Path.isSpecialFile
    pub fn is_special_file_with(&self, follow_symlinks: Symlinks) -> bool {
        self.file_system
            .is_special_file(&self.path_fragment, follow_symlinks.to_boolean())
    }

    /// Whether this is a symbolic link.
    // Java: com/google/devtools/build/lib/vfs/Path.java:441 Path.isSymbolicLink
    pub fn is_symbolic_link(&self) -> bool {
        self.file_system.is_symbolic_link(&self.path_fragment)
    }

    /// Opens for writing, truncating.
    // Java: com/google/devtools/build/lib/vfs/Path.java:452 Path.getOutputStream
    pub fn get_output_stream(&self) -> io::Result<Box<dyn Write + Send>> {
        self.file_system
            .get_output_stream(&self.path_fragment, false, false)
    }

    /// Opens for writing, appending or truncating.
    // Java: com/google/devtools/build/lib/vfs/Path.java:477 Path.getOutputStream
    pub fn get_output_stream_with(
        &self,
        append: bool,
        internal: bool,
    ) -> io::Result<Box<dyn Write + Send>> {
        self.file_system
            .get_output_stream(&self.path_fragment, append, internal)
    }

    /// Creates this directory; false if it already existed.
    // Java: com/google/devtools/build/lib/vfs/Path.java:488 Path.createDirectory
    pub fn create_directory(&self) -> io::Result<bool> {
        self.file_system.create_directory(&self.path_fragment)
    }

    /// Creates this directory and its parents.
    // Java: com/google/devtools/build/lib/vfs/Path.java:498 Path.createDirectoryAndParents
    pub fn create_directory_and_parents(&self) -> io::Result<()> {
        self.file_system
            .create_directory_and_parents(&self.path_fragment)
    }

    /// Creates a new temporary directory under this one.
    // Java: com/google/devtools/build/lib/vfs/Path.java:506 Path.createTempDirectory
    pub fn create_temp_directory(&self, prefix: &str) -> io::Result<Path> {
        let p = self
            .file_system
            .create_temp_directory(&self.path_fragment, prefix)?;
        Ok(Path::create(p, &self.file_system))
    }

    /// Makes this a symlink to `target` (same file system).
    // Java: com/google/devtools/build/lib/vfs/Path.java:534 Path.createSymbolicLink
    pub fn create_symbolic_link(&self, target: &Path) -> io::Result<()> {
        self.check_same_file_system(target);
        self.file_system
            .create_symbolic_link(&self.path_fragment, &target.path_fragment)
    }

    /// Makes this a symlink to the (absolute or relative) `target`.
    // Java: com/google/devtools/build/lib/vfs/Path.java:561 Path.createSymbolicLink
    pub fn create_symbolic_link_to_fragment(&self, target: &PathFragment) -> io::Result<()> {
        self.file_system
            .create_symbolic_link(&self.path_fragment, target)
    }

    /// The target of this symlink.
    // Java: com/google/devtools/build/lib/vfs/Path.java:577 Path.readSymbolicLink
    pub fn read_symbolic_link(&self) -> io::Result<PathFragment> {
        self.file_system.read_symbolic_link(&self.path_fragment)
    }

    /// The target of this symlink, assuming it is one.
    // Java: com/google/devtools/build/lib/vfs/Path.java:589 Path.readSymbolicLinkUnchecked
    pub fn read_symbolic_link_unchecked(&self) -> io::Result<PathFragment> {
        self.file_system
            .read_symbolic_link_unchecked(&self.path_fragment)
    }

    /// Creates `link` as a hard link to this file.
    // Java: com/google/devtools/build/lib/vfs/Path.java:599 Path.createHardLink
    pub fn create_hard_link(&self, link: &Path) -> io::Result<()> {
        self.file_system
            .create_hard_link(&link.path_fragment, &self.path_fragment)
    }

    /// The canonical path (realpath(3)).
    // Java: com/google/devtools/build/lib/vfs/Path.java:611 Path.resolveSymbolicLinks
    pub fn resolve_symbolic_links(&self) -> io::Result<Path> {
        let p = self
            .file_system
            .resolve_symbolic_links(&self.path_fragment)?;
        Ok(Path::create(p, &self.file_system))
    }

    /// Atomically renames this file to `target` (same file system).
    // Java: com/google/devtools/build/lib/vfs/Path.java:629 Path.renameTo
    pub fn rename_to(&self, target: &Path) -> io::Result<()> {
        self.check_same_file_system(target);
        self.file_system
            .rename_to(&self.path_fragment, &target.path_fragment)
    }

    /// Size in bytes, following symlinks.
    // Java: com/google/devtools/build/lib/vfs/Path.java:642 Path.getFileSize
    pub fn get_file_size(&self) -> io::Result<i64> {
        self.file_system.get_file_size(&self.path_fragment, true)
    }

    /// Size in bytes.
    // Java: com/google/devtools/build/lib/vfs/Path.java:657 Path.getFileSize
    pub fn get_file_size_with(&self, follow_symlinks: Symlinks) -> io::Result<i64> {
        self.file_system
            .get_file_size(&self.path_fragment, follow_symlinks.to_boolean())
    }

    /// Deletes this file (not following symlinks); true iff this call deleted it.
    // Java: com/google/devtools/build/lib/vfs/Path.java:670 Path.delete
    pub fn delete(&self) -> io::Result<bool> {
        self.file_system.delete(&self.path_fragment)
    }

    /// Deletes this directory tree.
    // Java: com/google/devtools/build/lib/vfs/Path.java:679 Path.deleteTree
    pub fn delete_tree(&self) -> io::Result<()> {
        self.file_system.delete_tree(&self.path_fragment)
    }

    /// Deletes everything beneath this directory.
    // Java: com/google/devtools/build/lib/vfs/Path.java:689 Path.deleteTreesBelow
    pub fn delete_trees_below(&self) -> io::Result<()> {
        self.file_system.delete_trees_below(&self.path_fragment)
    }

    /// Last modification time (ms since epoch), following symlinks.
    // Java: com/google/devtools/build/lib/vfs/Path.java:702 Path.getLastModifiedTime
    pub fn get_last_modified_time(&self) -> io::Result<i64> {
        self.file_system
            .get_last_modified_time(&self.path_fragment, true)
    }

    /// Last modification time (ms since epoch).
    // Java: com/google/devtools/build/lib/vfs/Path.java:717 Path.getLastModifiedTime
    pub fn get_last_modified_time_with(&self, follow_symlinks: Symlinks) -> io::Result<i64> {
        self.file_system
            .get_last_modified_time(&self.path_fragment, follow_symlinks.to_boolean())
    }

    /// Flushes the contents of the file to stable storage (`fsync`).
    ///
    /// PORT: Java syncs the `FileDescriptor` of an open `FileOutputStream` /
    /// `FileInputStream` (`LazyFileOutputStream.syncIfPossible`, `DiskCacheClient.syncFile`);
    /// the streams of this file system are opaque, and `fsync` acts on the file, so the file is
    /// synced through a descriptor of its own. Every file system here is the host file system.
    pub fn fsync(&self) -> io::Result<()> {
        std::fs::File::open(blaze_util::string_encoding::internal_to_platform(
            self.path_fragment.get_path_string(),
        ))
        .map_err(|e| crate::unix_file_system::posix_error(self.path_fragment.get_path_string(), e))?
        .sync_all()
    }

    /// Sets the modification time (ms since epoch, or [`NOW_SENTINEL_TIME`]).
    // Java: com/google/devtools/build/lib/vfs/Path.java:733 Path.setLastModifiedTime
    pub fn set_last_modified_time(&self, new_time: i64) -> io::Result<()> {
        self.file_system
            .set_last_modified_time(&self.path_fragment, new_time)
    }

    /// A digest obtainable without reading the file, if any.
    // Java: com/google/devtools/build/lib/vfs/Path.java:759 Path.getFastDigest
    pub fn get_fast_digest(&self) -> io::Result<Option<Vec<u8>>> {
        self.file_system.get_fast_digest(&self.path_fragment)
    }

    /// The digest of the file contents.
    // Java: com/google/devtools/build/lib/vfs/Path.java:773 Path.getDigest
    pub fn get_digest(&self) -> io::Result<Vec<u8>> {
        self.file_system.get_digest(&self.path_fragment)
    }

    /// Opens for reading.
    // Java: com/google/devtools/build/lib/vfs/Path.java:846 Path.getInputStream
    pub fn get_input_stream(&self) -> io::Result<Box<dyn Read + Send>> {
        self.file_system.get_input_stream(&self.path_fragment)
    }

    /// Whether the file is writable.
    // Java: com/google/devtools/build/lib/vfs/Path.java:876 Path.isWritable
    pub fn is_writable(&self) -> io::Result<bool> {
        self.file_system.is_writable(&self.path_fragment)
    }

    /// Sets readability.
    // Java: com/google/devtools/build/lib/vfs/Path.java:888 Path.setReadable
    pub fn set_readable(&self, readable: bool) -> io::Result<()> {
        self.file_system.set_readable(&self.path_fragment, readable)
    }

    /// Sets writability.
    // Java: com/google/devtools/build/lib/vfs/Path.java:902 Path.setWritable
    pub fn set_writable(&self, writable: bool) -> io::Result<()> {
        self.file_system.set_writable(&self.path_fragment, writable)
    }

    /// Whether the file is executable.
    // Java: com/google/devtools/build/lib/vfs/Path.java:914 Path.isExecutable
    pub fn is_executable(&self) -> io::Result<bool> {
        self.file_system.is_executable(&self.path_fragment)
    }

    /// Whether the file is readable.
    // Java: com/google/devtools/build/lib/vfs/Path.java:926 Path.isReadable
    pub fn is_readable(&self) -> io::Result<bool> {
        self.file_system.is_readable(&self.path_fragment)
    }

    /// Sets executability.
    // Java: com/google/devtools/build/lib/vfs/Path.java:938 Path.setExecutable
    pub fn set_executable(&self, executable: bool) -> io::Result<()> {
        self.file_system
            .set_executable(&self.path_fragment, executable)
    }

    /// Sets the permission bits.
    // Java: com/google/devtools/build/lib/vfs/Path.java:952 Path.chmod
    pub fn chmod(&self, mode: u32) -> io::Result<()> {
        self.file_system.chmod(&self.path_fragment, mode)
    }

    // Java: com/google/devtools/build/lib/vfs/Path.java:960 Path.checkSameFileSystem
    fn check_same_file_system(&self, that: &Path) {
        if fs_addr(&self.file_system) != fs_addr(&that.file_system) {
            panic!(
                "Files are on different filesystems: {} (on {:?}), {} (on {:?})",
                self, self.file_system, that, that.file_system
            );
        }
    }
}

// Java: com/google/devtools/build/lib/vfs/Path.java:213 Path.toString
impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.path_fragment.get_path_string())
    }
}

impl fmt::Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Path({:?})", self.path_fragment.get_path_string())
    }
}

// Java: com/google/devtools/build/lib/vfs/Path.java:218 Path.equals
impl PartialEq for Path {
    fn eq(&self, other: &Self) -> bool {
        fs_addr(&self.file_system) == fs_addr(&other.file_system)
            && self.path_fragment == other.path_fragment
    }
}

impl Eq for Path {}

// Java: com/google/devtools/build/lib/vfs/Path.java:229 Path.hashCode
impl Hash for Path {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Do not include file system for efficiency.
        self.path_fragment.hash(state);
    }
}

/// Paths on different file systems are ordered by file system address (Java uses
/// `System.identityHashCode`), then by fragment.
// Java: com/google/devtools/build/lib/vfs/Path.java:237 Path.compareTo
impl Ord for Path {
    fn cmp(&self, other: &Self) -> Ordering {
        fs_addr(&self.file_system)
            .cmp(&fs_addr(&other.file_system))
            .then_with(|| self.path_fragment.cmp(&other.path_fragment))
    }
}

impl PartialOrd for Path {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
