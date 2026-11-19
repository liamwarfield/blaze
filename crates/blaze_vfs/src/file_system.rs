//! Port of `FileSystem` (the abstract file system API) and its exception types.

use std::fmt;
use std::io::{self, Read, Write};
use std::sync::Arc;

use crate::digest_hash_function::DigestHashFunction;
use crate::dirent::{Dirent, DirentType};
use crate::file_status::FileStatus;
use crate::path::Path;
use crate::path_fragment::PathFragment;
use crate::root::Root;

/// Maximum number of symlinks followed by [`FileSystem::resolve_symbolic_links`].
// Java: com/google/devtools/build/lib/vfs/FileSystem.java:54 FileSystem.MAX_SYMLINKS
pub const MAX_SYMLINKS: i32 = 32;

// Java: com/google/devtools/build/lib/vfs/FileSystem.java:59 FileSystem.ERR_FILE_EXISTS
pub(crate) const ERR_FILE_EXISTS: &str = " (File exists)";
// Java: com/google/devtools/build/lib/vfs/FileSystem.java:62 FileSystem.ERR_NO_SUCH_FILE_OR_DIR
pub(crate) const ERR_NO_SUCH_FILE_OR_DIR: &str = " (No such file or directory)";

/// A shared handle to a file system. Paths compare file systems by identity (the `Arc` data
/// pointer), as Java compares `FileSystem` references.
pub type FileSystemRef = Arc<dyn FileSystem>;

/// Thrown when attempting to resolve an ordinary file as a symlink (Java
/// `FileSystem.NotASymlinkException`). Carried as the inner error of an [`io::Error`] with kind
/// `InvalidInput`; detect it with [`NotASymlinkException::is`].
// Java: com/google/devtools/build/lib/vfs/FileSystem.java:76 FileSystem.NotASymlinkException
#[derive(Debug, Clone)]
pub struct NotASymlinkException {
    message: String,
}

impl NotASymlinkException {
    /// Creates the error for `path`: "<path> is not a symlink".
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:77 FileSystem.NotASymlinkException.<init>
    pub fn new_error(path: &PathFragment) -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            NotASymlinkException {
                message: format!("{} is not a symlink", path.get_path_string()),
            },
        )
    }

    /// Whether `e` is a `NotASymlinkException`.
    pub fn is(e: &io::Error) -> bool {
        e.get_ref()
            .is_some_and(|r| r.downcast_ref::<NotASymlinkException>().is_some())
    }
}

impl fmt::Display for NotASymlinkException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for NotASymlinkException {}

/// A symlink chain too long or cyclic (Java `FileSymlinkLoopException`). Carried as the inner
/// error of an [`io::Error`]; detect it with [`FileSymlinkLoopException::is`].
// Java: com/google/devtools/build/lib/vfs/FileSymlinkLoopException.java:20 FileSymlinkLoopException
#[derive(Debug, Clone)]
pub struct FileSymlinkLoopException {
    message: String,
}

impl FileSymlinkLoopException {
    /// "<path> (Too many levels of symbolic links)".
    // Java: com/google/devtools/build/lib/vfs/FileSymlinkLoopException.java:25 FileSymlinkLoopException.<init>
    pub fn new_error(path: &PathFragment) -> io::Error {
        io::Error::other(FileSymlinkLoopException {
            message: format!(
                "{} (Too many levels of symbolic links)",
                path.get_path_string()
            ),
        })
    }

    /// The exception the native layer throws for `ELOOP`: `<path> (<strerror>)`.
    // Java: (native) src/main/native/unix_jni.cc PostException
    pub(crate) fn with_message(message: String) -> io::Error {
        io::Error::other(FileSymlinkLoopException { message })
    }

    /// Whether `e` is a `FileSymlinkLoopException` (or the native `ELOOP` error).
    pub fn is(e: &io::Error) -> bool {
        e.raw_os_error() == Some(ELOOP)
            || e.get_ref()
                .is_some_and(|r| r.downcast_ref::<FileSymlinkLoopException>().is_some())
    }
}

impl fmt::Display for FileSymlinkLoopException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FileSymlinkLoopException {}

pub(crate) const ELOOP: i32 = 40;
pub(crate) const ENOTDIR: i32 = 20;

/// Whether `e` means "not found" (ENOENT or ENOTDIR), as the native stat treats it.
pub(crate) fn is_not_found(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::NotFound || e.raw_os_error() == Some(ENOTDIR)
}

/// The abstract file system (Java `FileSystem`). Methods take absolute [`PathFragment`]s; use
/// [`Path`] for the object-oriented API.
///
/// Errors are [`io::Error`]s whose messages follow Bazel's native layer: `<path> (<strerror>)`.
// Java: com/google/devtools/build/lib/vfs/FileSystem.java:50 FileSystem
pub trait FileSystem: std::any::Any + Send + Sync + fmt::Debug {
    /// The digest function used for file contents.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:71 FileSystem.getDigestFunction
    fn get_digest_function(&self) -> DigestHashFunction;

    /// Whether the file system supports modifications (setWritable, setExecutable).
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:134 FileSystem.supportsModifications
    fn supports_modifications(&self, path: &PathFragment) -> bool;

    /// Whether the file system supports symbolic links natively.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:152 FileSystem.supportsSymbolicLinksNatively
    fn supports_symbolic_links_natively(&self, path: &PathFragment) -> bool;

    /// Whether the file system supports hard links natively.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:167 FileSystem.supportsHardLinksNatively
    fn supports_hard_links_natively(&self, path: &PathFragment) -> bool;

    /// Whether paths may be case- or normalization-insensitive.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:177 FileSystem.mayBeCaseOrNormalizationInsensitive
    fn may_be_case_or_normalization_insensitive(&self) -> bool;

    /// Creates a directory; returns false if it already existed as a directory.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:220 FileSystem.createDirectory
    fn create_directory(&self, path: &PathFragment) -> io::Result<bool>;

    /// Creates a directory and all missing parents.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:226 FileSystem.createDirectoryAndParents
    fn create_directory_and_parents(&self, path: &PathFragment) -> io::Result<()>;

    /// File size in bytes.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:236 FileSystem.getFileSize
    fn get_file_size(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<i64>;

    /// Deletes the file (not following symlinks). Returns true iff this call deleted it, false if
    /// it did not exist.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:239 FileSystem.delete
    fn delete(&self, path: &PathFragment) -> io::Result<bool>;

    /// Deletes all directory trees beneath `path` and `path` itself.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:247 FileSystem.deleteTree
    fn delete_tree(&self, path: &PathFragment) -> io::Result<()> {
        self.delete_trees_below(path)?;
        self.delete(path)?;
        Ok(())
    }

    /// Deletes all directory trees beneath `dir`. Does nothing if `dir` is not a directory.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:265 FileSystem.deleteTreesBelow
    fn delete_trees_below(&self, dir: &PathFragment) -> io::Result<()> {
        if self.is_directory(dir, false) {
            let entries = match self.get_directory_entries(dir) {
                Ok(entries) => entries,
                Err(_) => {
                    // If we couldn't read the directory, it may be because it's not readable. Try
                    // granting this permission and retry. If the retry fails, give up.
                    self.set_readable(dir, true)?;
                    self.set_executable(dir, true)?;
                    self.get_directory_entries(dir)?
                }
            };
            let mut iterator = entries.into_iter();
            if let Some(first_name) = iterator.next() {
                let first = dir.get_child(&first_name);
                self.delete_trees_below(&first)?;
                let result = match self.delete(&first) {
                    Ok(true) => Ok(()),
                    Ok(false) => Err(io::Error::other(format!(
                        "Unable to delete \"{}\": directory entry does not exist",
                        first
                    ))),
                    Err(e) => Err(e),
                };
                if result.is_err() {
                    // If we couldn't delete the first entry in a directory, it may be because the
                    // directory (not the entry!) is not writable or executable. Try granting this
                    // permission and retry.
                    self.set_writable(dir, true)?;
                    self.set_executable(dir, true)?;
                    self.delete_trees_below(&first)?;
                    self.delete(&first)?;
                }
            }
            for name in iterator {
                let path = dir.get_child(&name);
                self.delete_trees_below(&path)?;
                // No need to retry here: if needed, we already unprotected the directory earlier.
                self.delete(&path)?;
            }
        }
        Ok(())
    }

    /// Last modification time, milliseconds since the Unix epoch.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:319 FileSystem.getLastModifiedTime
    fn get_last_modified_time(&self, path: &PathFragment, follow_symlinks: bool)
    -> io::Result<i64>;

    /// Sets the last modification time (ms since epoch), or to now if `new_time` is
    /// [`crate::path::NOW_SENTINEL_TIME`].
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:326 FileSystem.setLastModifiedTime
    fn set_last_modified_time(&self, path: &PathFragment, new_time: i64) -> io::Result<()>;

    /// A digest available without reading the file (e.g. from an xattr), if any.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:355 FileSystem.getFastDigest
    fn get_fast_digest(&self, _path: &PathFragment) -> io::Result<Option<Vec<u8>>> {
        Ok(None)
    }

    /// The digest of the file contents, following symlinks.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:367 FileSystem.getDigest
    fn get_digest(&self, path: &PathFragment) -> io::Result<Vec<u8>> {
        let mut input = self.get_input_stream(path)?;
        self.get_digest_function().hash_reader(&mut input)
    }

    /// Returns the target of `path` if it is a symlink, or `None` if it exists but is not one.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:433 FileSystem.resolveOneLink
    fn resolve_one_link(&self, path: &PathFragment) -> io::Result<Option<PathFragment>> {
        match self.read_symbolic_link(path) {
            Ok(target) => Ok(Some(target)),
            Err(e) if NotASymlinkException::is(&e) => {
                // Not a symbolic link.  Check it exists.
                if !self.exists(path, false) {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("{}{}", path, ERR_NO_SUCH_FILE_OR_DIR),
                    ));
                }
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    /// Returns the canonical path for `path` (which must be absolute), as a fragment. Analogous
    /// to realpath(3). Java returns a `Path`; see [`Path::resolve_symbolic_links`].
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:456 FileSystem.resolveSymbolicLinks
    fn resolve_symbolic_links(&self, path: &PathFragment) -> io::Result<PathFragment> {
        assert!(path.is_absolute());
        match path.get_parent_directory() {
            None => Ok(path.clone()), // (root)
            Some(parent_node) => {
                let parent = self.resolve_symbolic_links(&parent_node)?;
                append_segment(self, parent, path.get_base_name(), MAX_SYMLINKS)
            }
        }
    }

    /// Returns the status of a file.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:467 FileSystem.stat
    fn stat(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<FileStatus>;

    /// Like `stat`, but returns `None` on failures.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:471 FileSystem.statNullable
    fn stat_nullable(&self, path: &PathFragment, follow_symlinks: bool) -> Option<FileStatus> {
        self.stat(path, follow_symlinks).ok()
    }

    /// Like `stat`, but returns `None` if the file is not found.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:486 FileSystem.statIfFound
    fn stat_if_found(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<FileStatus>> {
        match self.stat(path, follow_symlinks) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// True iff `path` is an existing regular or special file.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:498 FileSystem.isFile
    fn is_file(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        self.stat_nullable(path, follow_symlinks)
            .is_some_and(|s| s.is_file())
    }

    /// True iff `path` is an existing special file.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:507 FileSystem.isSpecialFile
    fn is_special_file(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        self.stat_nullable(path, follow_symlinks)
            .is_some_and(|s| s.is_special_file())
    }

    /// True iff `path` is an existing symbolic link.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:516 FileSystem.isSymbolicLink
    fn is_symbolic_link(&self, path: &PathFragment) -> bool {
        self.stat_nullable(path, false)
            .is_some_and(|s| s.is_symbolic_link())
    }

    /// True iff `path` is an existing directory.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:525 FileSystem.isDirectory
    fn is_directory(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        self.stat_nullable(path, follow_symlinks)
            .is_some_and(|s| s.is_directory())
    }

    /// Creates a symbolic link at `link_path` pointing to `target_fragment`. (Java's
    /// `SymlinkTargetType` hint only matters on Windows and is not ported.)
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:537 FileSystem.createSymbolicLink
    fn create_symbolic_link(
        &self,
        link_path: &PathFragment,
        target_fragment: &PathFragment,
    ) -> io::Result<()>;

    /// Returns the target of a symbolic link; a [`NotASymlinkException`] if it is not one.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:562 FileSystem.readSymbolicLink
    fn read_symbolic_link(&self, path: &PathFragment) -> io::Result<PathFragment>;

    /// Like `read_symbolic_link`, assuming `path` is a symlink.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:571 FileSystem.readSymbolicLinkUnchecked
    fn read_symbolic_link_unchecked(&self, path: &PathFragment) -> io::Result<PathFragment> {
        self.read_symbolic_link(path)
    }

    /// True iff `path` denotes an existing file of any kind.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:584 FileSystem.exists
    fn exists(&self, path: &PathFragment, follow_symlinks: bool) -> bool;

    /// Names of all entries of the directory `path` (following symlinks to it), in OS order.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:592 FileSystem.getDirectoryEntries
    fn get_directory_entries(&self, path: &PathFragment) -> io::Result<Vec<String>>;

    /// Names and types of all entries of the directory `path`.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:619 FileSystem.readdir
    fn readdir(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<Vec<Dirent>> {
        let children = self.get_directory_entries(path)?;
        let mut dirents = Vec::with_capacity(children.len());
        for child in children {
            let child_path = path.get_child(&child);
            let t = dirent_from_stat(self.stat_nullable(&child_path, follow_symlinks).as_ref());
            dirents.push(Dirent::new(child, t));
        }
        Ok(dirents)
    }

    /// Whether the file is readable.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:635 FileSystem.isReadable
    fn is_readable(&self, path: &PathFragment) -> io::Result<bool>;

    /// Sets the file readable or not.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:646 FileSystem.setReadable
    fn set_readable(&self, path: &PathFragment, readable: bool) -> io::Result<()>;

    /// Whether the file is writable.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:653 FileSystem.isWritable
    fn is_writable(&self, path: &PathFragment) -> io::Result<bool>;

    /// Sets the file writable or not.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:663 FileSystem.setWritable
    fn set_writable(&self, path: &PathFragment, writable: bool) -> io::Result<()>;

    /// Whether the file is executable.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:670 FileSystem.isExecutable
    fn is_executable(&self, path: &PathFragment) -> io::Result<bool>;

    /// Sets the file executable or not.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:682 FileSystem.setExecutable
    fn set_executable(&self, path: &PathFragment, executable: bool) -> io::Result<()>;

    /// Sets the permission bits of the file.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:695 FileSystem.chmod
    fn chmod(&self, path: &PathFragment, mode: u32) -> io::Result<()> {
        self.set_readable(path, (mode & 0o400) != 0)?;
        self.set_writable(path, (mode & 0o200) != 0)?;
        self.set_executable(path, (mode & 0o100) != 0)?;
        Ok(())
    }

    /// Opens the file for reading.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:708 FileSystem.getInputStream
    fn get_input_stream(&self, path: &PathFragment) -> io::Result<Box<dyn Read + Send>>;

    /// Opens the file for writing (truncating unless `append`). `internal` only affects
    /// profiling in Java.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:740 FileSystem.getOutputStream
    fn get_output_stream(
        &self,
        path: &PathFragment,
        append: bool,
        internal: bool,
    ) -> io::Result<Box<dyn Write + Send>>;

    /// Atomically renames `source_path` to `target_path`, not following symlinks.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:758 FileSystem.renameTo
    fn rename_to(&self, source_path: &PathFragment, target_path: &PathFragment) -> io::Result<()>;

    /// Creates a hard link at `link_path` for the file at `original_path`.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:768 FileSystem.createHardLink
    fn create_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        if !self.exists(original_path, true) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "File \"{}\" linked from \"{}\" does not exist",
                    original_path.get_base_name(),
                    link_path.get_base_name()
                ),
            ));
        }
        if self.exists(link_path, true) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "New link file \"{}\" already exists",
                    link_path.get_base_name()
                ),
            ));
        }
        self.create_fs_dependent_hard_link(link_path, original_path)
    }

    /// Creates a hard link without the existence checks of `create_hard_link`.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:794 FileSystem.createFSDependentHardLink
    fn create_fs_dependent_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()>;

    /// Creates a new temporary directory named `prefix` + a random number under `parent`, with
    /// mode 0700.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:888 FileSystem.createTempDirectory
    fn create_temp_directory(
        &self,
        parent: &PathFragment,
        prefix: &str,
    ) -> io::Result<PathFragment> {
        loop {
            let candidate = parent.get_relative_str(&format!("{}{}", prefix, random_u64()));
            if self.create_directory(&candidate)? {
                self.chmod(&candidate, 0o700)?;
                return Ok(candidate);
            }
        }
    }
}

// Java uses SecureRandom; a randomly keyed hash of time and a counter suffices for unique names.
fn random_u64() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    if let Ok(d) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        h.write_u128(d.as_nanos());
    }
    h.finish()
}

/// Maps a status to a dirent type.
// Java: com/google/devtools/build/lib/vfs/FileSystem.java:594 FileSystem.direntFromStat
pub fn dirent_from_stat(stat: Option<&FileStatus>) -> DirentType {
    match stat {
        None => DirentType::Unknown,
        Some(s) if s.is_special_file() => DirentType::Unknown,
        Some(s) if s.is_file() => DirentType::File,
        Some(s) if s.is_directory() => DirentType::Directory,
        Some(s) if s.is_symbolic_link() => DirentType::Symlink,
        Some(_) => DirentType::Unknown,
    }
}

/// Appends `child` to the canonical directory `dir`, resolving it if it is a symlink.
// Java: com/google/devtools/build/lib/vfs/FileSystem.java:388 FileSystem.appendSegment
fn append_segment<F: FileSystem + ?Sized>(
    fs: &F,
    mut dir: PathFragment,
    child: &str,
    mut max_links: i32,
) -> io::Result<PathFragment> {
    let naive = dir.get_child(child);

    let link_target = match fs.resolve_one_link(&naive)? {
        None => return Ok(naive), // regular file or directory
        Some(t) => t,
    };

    if max_links == 0 {
        return Err(FileSymlinkLoopException::new_error(&naive));
    }
    max_links -= 1;
    if link_target.is_absolute() {
        dir = PathFragment::create_already_normalized(link_target.get_drive_str());
    }
    for name in link_target.segments() {
        if name == "." || name.is_empty() {
            // no-op
        } else if name == ".." {
            // root's parent is root, when canonicalizing, so this is a no-op.
            if let Some(parent) = dir.get_parent_directory() {
                dir = parent;
            }
        } else {
            dir = append_segment(fs, dir, name, max_links)?;
        }
    }
    Ok(dir)
}

impl dyn FileSystem {
    /// This file system as `Any`, for Java's `fs instanceof <FileSystem subclass>` checks
    /// (`downcast_ref::<T>()`).
    ///
    /// PORT: an inherent method of the trait object (trait upcasting), so every implementation
    /// has it without writing it; call it on the `dyn FileSystem` (an `Arc<dyn FileSystem>`
    /// derefs to it).
    pub fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Java's `FileSystem.getPath` and `getAbsoluteRoot`, which need the shared handle.
pub trait FileSystemExt {
    /// Returns the absolute path for `path` (normalized, no IO).
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:94 FileSystem.getPath
    fn get_path(&self, path: &str) -> Path;

    /// Returns the path for an absolute fragment.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:99 FileSystem.getPath
    fn get_path_fragment(&self, path_fragment: &PathFragment) -> Path;

    /// The absolute root of this file system.
    // Java: com/google/devtools/build/lib/vfs/FileSystem.java:103 FileSystem.getAbsoluteRoot
    fn absolute_root(&self) -> Root;
}

impl FileSystemExt for FileSystemRef {
    fn get_path(&self, path: &str) -> Path {
        Path::create_from_str(path, self)
    }

    fn get_path_fragment(&self, path_fragment: &PathFragment) -> Path {
        Path::create(path_fragment.clone(), self)
    }

    fn absolute_root(&self) -> Root {
        Root::absolute_root(self)
    }
}

/// Identity of a file system handle (the `Arc` data pointer), Java reference identity.
pub(crate) fn fs_addr(fs: &FileSystemRef) -> usize {
    Arc::as_ptr(fs) as *const () as usize
}
