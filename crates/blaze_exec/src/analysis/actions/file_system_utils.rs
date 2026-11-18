//! The subset of `com.google.devtools.build.lib.vfs.FileSystemUtils` that action execution uses.
//!
//! PORT: `FileSystemUtils` belongs to `blaze_vfs`, which this module does not own; these are
//! local ports of the individual methods, operating on [`blaze_vfs::Path`].

use std::io::{self, Read, Write};

use blaze_vfs::{Path, PathFragment};

/// Creates an empty regular file (truncating an existing one).
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:283 FileSystemUtils.createEmptyFile
pub fn create_empty_file(path: &Path) -> io::Result<()> {
    path.get_output_stream()?.flush()
}

/// Creates or updates a symbolic link from `link` to `target`. Replaces an existing symbolic link
/// with a different target; creates missing parent directories.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:329 FileSystemUtils.ensureSymbolicLink
pub fn ensure_symbolic_link(link: &Path, target: &PathFragment) -> io::Result<()> {
    // TODO(bazel-team): (2009) consider adding the logic for recovering from the case when we
    // have already created a parent directory symlink earlier.
    let mut parent_known_to_exist = false;
    match link.read_symbolic_link() {
        Ok(existing) => {
            if &existing == target {
                return Ok(());
            }
            link.delete()?;
            parent_known_to_exist = true;
        }
        // Path does not exist; fall through.
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        // NotASymlinkException and other I/O errors propagate.
        Err(e) => return Err(e),
    }
    if !parent_known_to_exist && let Some(parent) = link.get_parent_directory() {
        parent.create_directory_and_parents()?;
    }
    link.create_symbolic_link_to_fragment(target)
}

/// Writes the specified byte array to the output file. The file is created if it doesn't already
/// exist. If it does exist, it is overwritten.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:711 FileSystemUtils.writeContent
pub fn write_content(output_file: &Path, content: &[u8]) -> io::Result<()> {
    let mut out = output_file.get_output_stream()?;
    out.write_all(content)?;
    out.flush()
}

/// Updates the contents of the output file if they do not match the given array, thus
/// maintaining the mtime and ctime in case of no updates.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:745 FileSystemUtils.maybeUpdateContent
pub fn maybe_update_content(output_file: &Path, new_content: &[u8]) -> io::Result<()> {
    // Ignore read errors: what we are doing here is for performance reasons only, so we should
    // only break if the real action (that is, the write) fails.
    let current_content = read_content(output_file).ok();

    match current_content {
        None => write_content(output_file, new_content)?,
        Some(current_content) => {
            if new_content != current_content.as_slice() {
                if !output_file.is_writable()? {
                    output_file.delete()?;
                }
                write_content(output_file, new_content)?;
            }
        }
    }
    Ok(())
}

/// Returns the entire content of the file.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:814 FileSystemUtils.readContent
pub fn read_content(input_file: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    input_file.get_input_stream()?.read_to_end(&mut bytes)?;
    Ok(bytes)
}
