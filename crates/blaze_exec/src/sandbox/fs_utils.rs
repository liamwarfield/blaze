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

/// Copies all dir trees under a given 'from' dir to location 'to', while overwriting all files in
/// the potentially existing 'to'. Resolves symbolic links for files, and preserves them as
/// symlinks otherwise. Panics (Java `IllegalArgumentException`) if `to` is under `from`.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:575 FileSystemUtils.copyTreesBelow
pub fn copy_trees_below(from: &Path, to: &Path) -> io::Result<()> {
    if to.starts_with(from) {
        panic!("java.lang.IllegalArgumentException: {to} is a subdirectory of {from}");
    }

    for dirent in from.readdir(blaze_vfs::Symlinks::Nofollow)? {
        let from_child = from.get_child(dirent.get_name());
        let to_child = to.get_child(dirent.get_name());
        match dirent.get_type() {
            blaze_vfs::DirentType::File => copy_file(&from_child, &to_child)?,
            blaze_vfs::DirentType::Symlink => {
                ensure_symbolic_link(&to_child, &from_child.read_symbolic_link()?)?
            }
            blaze_vfs::DirentType::Directory => {
                to_child.create_directory()?;
                copy_trees_below(&from_child, &to_child)?;
            }
            _ => {
                return Err(io::Error::other(format!(
                    "Don't know how to copy {from_child}"
                )));
            }
        }
    }
    Ok(())
}

/// Creates a symbolic link `link` to `target`, creating the parent directories, unless it already
/// is a symlink to `target`; a symlink to another target is replaced. Fails if `link` exists and
/// is not a symlink.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:329 FileSystemUtils.ensureSymbolicLink
pub fn ensure_symbolic_link(link: &Path, target: &blaze_vfs::PathFragment) -> io::Result<()> {
    // TODO(bazel-team): (2009) consider adding the logic for recovering from the case when
    // we have already created a parent directory symlink earlier.
    let mut parent_known_to_exist = false;
    // This will throw if the path already exists and is not a symbolic link.
    match link.read_symbolic_link() {
        Ok(existing) => {
            if &existing == target {
                // Nothing to do.
                return Ok(());
            }
            // The symlink exists, but points elsewhere.
            link.delete()?;
            parent_known_to_exist = true;
        }
        // Path does not exist; fall through.
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    if !parent_known_to_exist {
        link.get_parent_directory()
            .expect("NullPointerException: link has no parent")
            .create_directory_and_parents()?;
    }
    link.create_symbolic_link_to_fragment(target)
}
