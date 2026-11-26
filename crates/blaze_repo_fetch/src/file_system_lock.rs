//! Port of `com.google.devtools.build.lib.util.FileSystemLock`.
//!
//! Java's `FileChannel.lock(0, Long.MAX_VALUE, shared)` takes a POSIX record lock (`fcntl`,
//! whole file) on Linux, so Blaze does the same to interoperate with a Bazel server sharing the
//! repository cache.
//!
//! A POSIX record lock belongs to the process, not to the descriptor: a second lock request on
//! the same file from the same process always succeeds, and closing *any* descriptor of the file
//! releases every lock the process holds on it. The JVM has the same kind of lock and guards it
//! with a per-JVM table: a second, overlapping `lock`/`tryLock` in the same JVM throws
//! `OverlappingFileLockException`. In Bazel that never happens for the repo contents cache (one
//! `LocalRepoContentsCache` per server, one command at a time).
//!
//! PORT: Blaze keeps the same per-process table ([`HELD`]) with one difference: a shared lock
//! on a file this process already holds shared is counted instead of refused, because Blaze
//! builds one `LocalRepoContentsCache` per command setup and a process may set up more than once
//! (tests, the dev drivers). The file stays locked until the last holder closes; without the
//! table the first close would release it for all of them (review 2026-11-18 round 2, N3). Any
//! other overlap in one process is Java's `OverlappingFileLockException`.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::sync::Mutex;

use blaze_util::string_encoding::internal_to_platform;
use blaze_vfs::Path;

use crate::errors::IoException;

/// The lock mode.
// Java: com/google/devtools/build/lib/util/FileSystemLock.java:47 FileSystemLock.LockMode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockMode {
    SHARED,
    EXCLUSIVE,
}

impl std::fmt::Display for LockMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            LockMode::SHARED => "shared",
            LockMode::EXCLUSIVE => "exclusive",
        })
    }
}

/// The lock file's identity: device and inode.
type FileKey = (u64, u64);

struct Held {
    /// The descriptor that carries the lock. Closing it releases the lock.
    _file: File,
    shared: bool,
    /// The number of [`FileSystemLock`]s for this file.
    holders: usize,
}

/// The file locks of this process (the JVM's `FileLockTable`).
static HELD: Mutex<Option<HashMap<FileKey, Held>>> = Mutex::new(None);

fn held() -> std::sync::MutexGuard<'static, Option<HashMap<FileKey, Held>>> {
    HELD.lock().unwrap_or_else(|e| e.into_inner())
}

/// A held file lock; released on drop (Java `close`).
// Java: com/google/devtools/build/lib/util/FileSystemLock.java:33 FileSystemLock
#[derive(Debug)]
pub struct FileSystemLock {
    key: FileKey,
    shared: bool,
}

fn prepare_channel(path: &Path) -> Result<File, IoException> {
    if let Some(parent) = path.get_parent_directory() {
        parent.create_directory_and_parents()?;
    }
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(internal_to_platform(path.get_path_string()))?)
}

fn fcntl_lock(file: &File, mode: LockMode, wait: bool) -> std::io::Result<bool> {
    // SAFETY: flock is plain-old-data; zeroed is a valid initial value.
    let mut fl: libc::flock = unsafe { std::mem::zeroed() };
    fl.l_type = match mode {
        LockMode::SHARED => libc::F_RDLCK as libc::c_short,
        LockMode::EXCLUSIVE => libc::F_WRLCK as libc::c_short,
    };
    fl.l_whence = libc::SEEK_SET as libc::c_short;
    fl.l_start = 0;
    // Java maps a size of Long.MAX_VALUE to l_len = 0 ("to end of file and beyond").
    fl.l_len = 0;
    let cmd = if wait { libc::F_SETLKW } else { libc::F_SETLK };
    loop {
        // SAFETY: valid fd and a valid pointer to an initialized flock struct.
        let r = unsafe { libc::fcntl(file.as_raw_fd(), cmd, &fl) };
        if r == 0 {
            return Ok(true);
        }
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EINTR) if wait => continue,
            Some(libc::EAGAIN) | Some(libc::EACCES) if !wait => return Ok(false),
            _ => return Err(err),
        }
    }
}

impl FileSystemLock {
    /// Tries to acquire the lock without blocking.
    // Java: com/google/devtools/build/lib/util/FileSystemLock.java:94 FileSystemLock.tryGet
    pub fn try_get(path: &Path, mode: LockMode) -> Result<FileSystemLock, IoException> {
        match Self::acquire(path, mode, false)? {
            Some(lock) => Ok(lock),
            None => Err(IoException::new(format!(
                "failed to acquire {} filesystem lock on {}",
                mode, path
            ))),
        }
    }

    /// Acquires the lock, blocking.
    // Java: com/google/devtools/build/lib/util/FileSystemLock.java:117 FileSystemLock.get
    pub fn get(path: &Path, mode: LockMode) -> Result<FileSystemLock, IoException> {
        Ok(Self::acquire(path, mode, true)?.expect("a blocking lock request does not fail"))
    }

    /// `None` when the lock is held by another process and `wait` is false.
    ///
    /// PORT: the process table stays locked while this waits for another process, so lock
    /// requests of other threads wait too. Blaze takes these locks once per command.
    fn acquire(
        path: &Path,
        mode: LockMode,
        wait: bool,
    ) -> Result<Option<FileSystemLock>, IoException> {
        let mut held = held();
        let table = held.get_or_insert_with(HashMap::new);
        let shared = mode == LockMode::SHARED;
        // Look the file up without opening it: closing a descriptor of a file this process has
        // locked would release that lock.
        if let Ok(metadata) = std::fs::metadata(internal_to_platform(path.get_path_string())) {
            let key = (metadata.dev(), metadata.ino());
            if let Some(entry) = table.get_mut(&key) {
                if entry.shared && shared {
                    entry.holders += 1;
                    return Ok(Some(FileSystemLock { key, shared }));
                }
                // sun.nio.ch.FileLockTable.checkList, from FileChannelImpl.lock / tryLock.
                panic!("java.nio.channels.OverlappingFileLockException");
            }
        }
        let file = prepare_channel(path)?;
        let metadata = file.metadata()?;
        let key = (metadata.dev(), metadata.ino());
        if !fcntl_lock(&file, mode, wait)? {
            return Ok(None);
        }
        table.insert(
            key,
            Held {
                _file: file,
                shared,
                holders: 1,
            },
        );
        Ok(Some(FileSystemLock { key, shared }))
    }

    pub fn is_shared(&self) -> bool {
        self.shared
    }

    /// Whether this process holds a lock on `path` (Java: the JVM's `FileLockTable` has an
    /// entry, so a further overlapping request would throw `OverlappingFileLockException`).
    ///
    /// PORT: Blaze-only, for the repo contents cache garbage collector of a server whose kept
    /// graph still holds the cache's shared lock (`blaze_runtime` `idle_tasks.rs`).
    pub fn held_by_this_process(path: &Path) -> bool {
        let Ok(metadata) = std::fs::metadata(internal_to_platform(path.get_path_string())) else {
            return false;
        };
        held()
            .as_ref()
            .is_some_and(|table| table.contains_key(&(metadata.dev(), metadata.ino())))
    }

    pub fn is_exclusive(&self) -> bool {
        !self.shared
    }

    /// Releases the lock.
    // Java: com/google/devtools/build/lib/util/FileSystemLock.java:150 FileSystemLock.close
    pub fn close(self) {}
}

impl Drop for FileSystemLock {
    fn drop(&mut self) {
        let mut held = held();
        let Some(table) = held.as_mut() else {
            return;
        };
        if let Some(entry) = table.get_mut(&self.key) {
            entry.holders -= 1;
            if entry.holders == 0 {
                // Closes the descriptor, which releases the lock.
                table.remove(&self.key);
            }
        }
    }
}
