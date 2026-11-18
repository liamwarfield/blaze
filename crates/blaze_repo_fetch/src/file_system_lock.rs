//! Port of `com.google.devtools.build.lib.util.FileSystemLock`.
//!
//! Java's `FileChannel.lock(0, Long.MAX_VALUE, shared)` takes a POSIX record lock (`fcntl`,
//! whole file) on Linux, so Blaze does the same to interoperate with a Bazel server sharing the
//! repository cache.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;

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

/// A held file lock; released on drop (Java `close`).
// Java: com/google/devtools/build/lib/util/FileSystemLock.java:33 FileSystemLock
#[derive(Debug)]
pub struct FileSystemLock {
    file: File,
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
        .open(path.get_path_string())?)
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
        let file = prepare_channel(path)?;
        if !fcntl_lock(&file, mode, false)? {
            return Err(IoException::new(format!(
                "failed to acquire {} filesystem lock on {}",
                mode, path
            )));
        }
        Ok(FileSystemLock {
            file,
            shared: mode == LockMode::SHARED,
        })
    }

    /// Acquires the lock, blocking.
    // Java: com/google/devtools/build/lib/util/FileSystemLock.java:117 FileSystemLock.get
    pub fn get(path: &Path, mode: LockMode) -> Result<FileSystemLock, IoException> {
        let file = prepare_channel(path)?;
        fcntl_lock(&file, mode, true)?;
        Ok(FileSystemLock {
            file,
            shared: mode == LockMode::SHARED,
        })
    }

    pub fn is_shared(&self) -> bool {
        self.shared
    }

    pub fn is_exclusive(&self) -> bool {
        !self.shared
    }

    /// Releases the lock.
    // Java: com/google/devtools/build/lib/util/FileSystemLock.java:150 FileSystemLock.close
    pub fn close(self) {
        drop(self.file);
    }
}
