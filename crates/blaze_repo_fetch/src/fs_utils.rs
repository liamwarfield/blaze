//! The subset of `com.google.devtools.build.lib.vfs.FileSystemUtils` the fetch code uses.
//!
//! PORT: `FileSystemUtils` belongs to `blaze_vfs`, which this crate does not own; these are
//! local ports of the individual methods, operating on [`blaze_vfs::Path`].

use std::io::{self, Read, Write};

use blaze_util::string_encoding::java_new_string_utf8;
use blaze_util::string_encoding::latin1_decode;
use blaze_util::string_encoding::latin1_encode_lossy;
use blaze_vfs::{NOW_SENTINEL_TIME, Path, PathFragment, Symlinks};

use crate::errors::IoException;

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

/// Creates or updates a symlink from `link` to `target`, creating missing parent directories.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:329 FileSystemUtils.ensureSymbolicLink
pub fn ensure_symbolic_link(link: &Path, target: &PathFragment) -> io::Result<()> {
    let mut parent_known_to_exist = false;
    match link.read_symbolic_link() {
        Ok(existing) => {
            if &existing == target {
                return Ok(());
            }
            link.delete()?;
            parent_known_to_exist = true;
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    if !parent_known_to_exist && let Some(parent) = link.get_parent_directory() {
        parent.create_directory_and_parents()?;
    }
    link.create_symbolic_link_to_fragment(target)
}

/// Copies a regular file (following symlinks), preserving mtime and permissions.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:387 FileSystemUtils.copyFile
pub fn copy_file(from: &Path, to: &Path) -> io::Result<()> {
    let stat = from.stat()?;
    copy_file_with_stat(from, to, &stat)
}

// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:391 FileSystemUtils.copyFile
fn copy_file_with_stat(from: &Path, to: &Path, stat: &blaze_vfs::FileStatus) -> io::Result<()> {
    if !stat.is_file() {
        return Err(io::Error::other(format!("don't know how to copy {}", from)));
    }
    // PORT: Java's fast path is `Files.copy(REPLACE_EXISTING, COPY_ATTRIBUTES)`, which replaces
    // a symlink at the destination and keeps mtime and permissions; the slow path below does
    // the same through the VFS.
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

/// Renames `source` to `target`; on Linux this is a plain atomic rename.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:509 FileSystemUtils.renameToleratingConcurrentCreation
pub fn rename_tolerating_concurrent_creation(source: &Path, target: &Path) -> io::Result<()> {
    // PORT: skipped, Windows only (the FileAccessException fallback).
    source.rename_to(target)
}

/// Moves a file, falling back to copy-and-delete when renaming fails.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:425 FileSystemUtils.moveFile
pub fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    if from.rename_to(to).is_ok() {
        return Ok(());
    }
    let stat = from.stat_with(Symlinks::Nofollow)?;
    if stat.is_file() {
        copy_file_with_stat(from, to, &stat)?;
    } else if stat.is_symbolic_link() {
        let target_path = from.read_symbolic_link()?;
        if to.create_symbolic_link_to_fragment(&target_path).is_err() {
            to.delete()?;
            to.create_symbolic_link_to_fragment(&target_path)?;
        }
    } else {
        return Err(io::Error::other(format!("Don't know how to move {}", from)));
    }
    if let Err(e) = from.delete() {
        let _ = to.delete();
        return Err(e);
    }
    Ok(())
}

/// Moves all entries below `from` to below `to`.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:482 FileSystemUtils.moveTreesBelow
pub fn move_trees_below(from: &Path, to: &Path) -> io::Result<()> {
    if to.starts_with(from) {
        panic!(
            "IllegalArgumentException: {} is a subdirectory of {}",
            to, from
        );
    }
    from.chmod(0o755)?;
    for entry in from.get_directory_entries()? {
        if entry.is_directory_with(Symlinks::Nofollow) {
            let sub_dir = to.get_child(entry.get_base_name());
            sub_dir.create_directory()?;
            move_trees_below(&entry, &sub_dir)?;
        } else {
            let new_entry = to.get_child(entry.get_base_name());
            move_file(&entry, &new_entry)?;
        }
    }
    Ok(())
}

/// Writes bytes to the file (following symlinks), truncating it.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:711 FileSystemUtils.writeContent
pub fn write_content(output_file: &Path, content: &[u8]) -> io::Result<()> {
    let mut out = output_file.get_output_stream()?;
    out.write_all(content)?;
    out.flush()
}

/// Writes lines, each terminated by `\n`, encoded as UTF-8 (`writeLinesAs(file, UTF_8, lines)`),
/// creating parent directories.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:723 FileSystemUtils.writeLinesAs
pub fn write_lines_as(file: &Path, lines: &[String]) -> io::Result<()> {
    if let Some(parent) = file.get_parent_directory() {
        parent.create_directory_and_parents()?;
    }
    let mut content = String::new();
    for line in lines {
        content.push_str(line);
        content.push('\n');
    }
    write_content(file, content.as_bytes())
}

/// Reads the whole file.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:814 FileSystemUtils.readContent
pub fn read_content(input_file: &Path) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    input_file.get_input_stream()?.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Reads the file as UTF-8 lines (`readLines(file, UTF_8)`, Guava `CharSource.readLines`):
/// lines end at `\n`, `\r\n` or `\r`, terminators are dropped, and a trailing terminator does
/// not add an empty line. The lines are Unicode strings; malformed UTF-8 is replaced with U+FFFD
/// as the JDK's decoder does.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:804 FileSystemUtils.readLines
pub fn read_lines(input_file: &Path) -> io::Result<Vec<String>> {
    let content = read_content(input_file)?;
    let text = java_new_string_utf8(&content);
    Ok(split_lines(&text))
}

/// Guava `LineReader` line splitting.
pub fn split_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    let mut pending = false;
    while let Some(c) = chars.next() {
        match c {
            '\n' => {
                lines.push(std::mem::take(&mut current));
                pending = false;
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                lines.push(std::mem::take(&mut current));
                pending = false;
            }
            _ => {
                current.push(c);
                pending = true;
            }
        }
    }
    if pending {
        lines.push(current);
    }
    lines
}

/// Creates a hard link at `link_path` for `original_path`, recursing into directories.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:937 FileSystemUtils.createHardLink
pub fn create_hard_link(link_path: &Path, original_path: &Path) -> io::Result<()> {
    if original_path.is_directory() {
        for original_subpath in original_path.get_directory_entries()? {
            let link_subpath = link_path.get_relative(&original_subpath.relative_to(original_path));
            create_hard_link(&link_subpath, &original_subpath)?;
        }
    } else {
        if let Some(parent_dir) = link_path.get_parent_directory()
            && !parent_dir.exists()
        {
            parent_dir.create_directory_and_parents()?;
        }
        original_path.create_hard_link(link_path)?;
    }
    Ok(())
}

/// The internal string under which the file system sees `path` when Java hands it a string
/// that is not internal (a Unicode string, e.g. a name decoded from UTF-8 or US-ASCII) without
/// converting it: the JNI (`GetStringLatin1Chars`) passes a char <= U+00FF as that byte and any
/// other char as `?`. So a Unicode `é` (U+00E9) names a file with the single byte E9.
// Java: src/main/native/latin1_jni_path.cc GetStringLatin1Chars
pub fn unicode_path_as_seen_by_file_system(path: &str) -> String {
    latin1_decode(&latin1_encode_lossy(path)).into_owned()
}

/// Converts a VFS result into the Java exception type.
pub(crate) fn io<T>(r: io::Result<T>) -> Result<T, IoException> {
    r.map_err(IoException::from)
}
