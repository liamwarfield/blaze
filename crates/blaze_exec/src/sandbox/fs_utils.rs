//! The subset of `com.google.devtools.build.lib.vfs.FileSystemUtils` the sandbox code uses.
//!
//! PORT: `FileSystemUtils` belongs to `blaze_vfs`, which this crate does not own; these are local
//! ports of the individual methods, operating on [`blaze_vfs::Path`].

use std::io::{self, Write};

use blaze_vfs::{FileStatus, NOW_SENTINEL_TIME, Path};

/// "Touches" the file, following symlinks: creates it empty or sets its mtime to now.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:269 FileSystemUtils.touchFile
pub fn touch_file(path: &Path) -> io::Result<()> {
    if path.exists() {
        path.set_last_modified_time(NOW_SENTINEL_TIME)
    } else {
        create_empty_file(path)
    }
}

/// Creates an empty regular file.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:283 FileSystemUtils.createEmptyFile
pub fn create_empty_file(path: &Path) -> io::Result<()> {
    path.get_output_stream()?.flush()
}

/// Copies a regular file (following symlinks), preserving mtime and permissions.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:387 FileSystemUtils.copyFile
pub fn copy_file(from: &Path, to: &Path) -> io::Result<()> {
    let stat = from.stat()?;
    copy_file_with_stat(from, to, &stat)
}

// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:391 FileSystemUtils.copyFile
fn copy_file_with_stat(from: &Path, to: &Path, stat: &FileStatus) -> io::Result<()> {
    if !stat.is_file() {
        return Err(io::Error::other(format!("don't know how to copy {}", from)));
    }
    // PORT: Java's fast path is `Files.copy(REPLACE_EXISTING, COPY_ATTRIBUTES)`, which replaces a
    // symlink at the destination and keeps mtime and permissions; the slow path below does the
    // same through the VFS.
    to.delete()?;
    {
        let mut input = from.get_input_stream()?;
        let mut out = to.get_output_stream()?;
        io::copy(&mut input, &mut out)?;
        out.flush()?;
    }
    to.set_last_modified_time(stat.get_last_modified_time())?;
    to.chmod(stat.get_permissions())?;
    Ok(())
}
