//! Port of `SevenZDecompressor`. This generally follows the logic of `ZipDecompressor`, except
//! that the 7z format does not support file permissions or symbolic links.
//!
//! PORT: Java reads with Commons Compress `SevenZFile`; Blaze uses `sevenz-rust2`.

use std::io::{self, Read, Write as _};

use blaze_vfs::{Path, PathFragment};

use super::{
    DecompressorDescriptor, StripPrefixedPath, could_not_find_prefix_exception, hash_set_prefixes,
    maybe_make_prefix_suggestion,
};
use crate::errors::{DecompressorError, IoException};

fn sevenz_error(e: sevenz_rust2::Error) -> IoException {
    match e {
        sevenz_rust2::Error::Io(io_err, _) => io_err.into(),
        other => IoException::new(other.to_string()),
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/SevenZDecompressor.java:45 SevenZDecompressor.decompress
pub(crate) fn decompress(descriptor: &DecompressorDescriptor) -> Result<Path, DecompressorError> {
    let destination_directory = &descriptor.destination_path;
    let prefix = descriptor.prefix.as_deref();
    let rename_files = &descriptor.rename_files;
    let mut found_prefix = false;

    let file = std::fs::File::open(descriptor.archive_path.get_path_string())?;
    let mut reader = sevenz_rust2::ArchiveReader::new(file, sevenz_rust2::Password::empty())
        .map_err(sevenz_error)?;
    let names: Vec<String> = reader
        .archive()
        .files
        .iter()
        .map(|e| e.name.clone())
        .collect();
    // From the Commons Compress docs: some 7z archives don't contain any names for the archive
    // entries. We throw an error for ALL nameless entries.
    let mut error: Option<IoException> = None;
    reader
        .for_each_entries(|entry, data| {
            let result = (|| -> Result<(), IoException> {
                let mut entry_name = entry.name().to_string();
                if entry_name.is_empty() {
                    return Err(IoException::new("7z archive contains unnamed entry"));
                }
                if let Some(r) = rename_files.get(&entry_name) {
                    entry_name = r.clone();
                }
                let entry_path = StripPrefixedPath::maybe_deprefix(entry_name.as_bytes(), prefix);
                found_prefix = found_prefix || entry_path.found_prefix();
                if entry_path.skip() {
                    return Ok(());
                }
                let path_fragment = entry_path
                    .get_path_fragment()
                    .strip_components(descriptor.strip_components);
                if path_fragment.is_empty() {
                    return Ok(());
                }
                extract_7z_entry(entry, data, destination_directory, &path_fragment)
            })();
            match result {
                Ok(()) => Ok(true),
                Err(e) => {
                    error = Some(e);
                    Err(sevenz_rust2::Error::Other("aborted".into()))
                }
            }
        })
        .map_err(|e| match error.take() {
            Some(err) => err,
            None => sevenz_error(e),
        })?;

    if let Some(prefix) = prefix
        && !found_prefix
    {
        let mut prefixes = Vec::new();
        for name in &names {
            let entry_path = StripPrefixedPath::maybe_deprefix(name.as_bytes(), None);
            if let Some(s) = maybe_make_prefix_suggestion(entry_path.get_path_fragment()) {
                prefixes.push(s);
            }
        }
        return Err(could_not_find_prefix_exception(prefix, &hash_set_prefixes(prefixes)).into());
    }
    Ok(destination_directory.clone())
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/SevenZDecompressor.java:117 SevenZDecompressor.extract7zEntry
fn extract_7z_entry(
    entry: &sevenz_rust2::ArchiveEntry,
    data: &mut dyn Read,
    destination_directory: &Path,
    stripped_relative_path: &PathFragment,
) -> Result<(), IoException> {
    if stripped_relative_path.is_absolute() {
        return Err(IoException::new(format!(
            "Failed to extract {}, 7-zipped paths cannot be absolute",
            stripped_relative_path
        )));
    }
    // Sanity/security check - at this point, uplevel references (..) should be resolved.
    if stripped_relative_path.contains_uplevel_references() {
        return Err(IoException::new(format!(
            "Failed to extract {}, 7-zipped entry contains uplevel references (..)",
            stripped_relative_path
        )));
    }
    let output_path = destination_directory.get_relative(stripped_relative_path);
    if let Some(parent) = output_path.get_parent_directory() {
        parent.create_directory_and_parents()?;
    }
    if entry.is_directory() {
        output_path.create_directory_and_parents()?;
    } else {
        {
            let mut output = output_path.get_output_stream()?;
            io::copy(data, &mut output)?;
            output.flush()?;
        }
        if entry.has_last_modified_date {
            let t: std::time::SystemTime = entry.last_modified_date().into();
            let millis = match t.duration_since(std::time::UNIX_EPOCH) {
                Ok(d) => d.as_millis() as i64,
                Err(e) => -(e.duration().as_millis() as i64),
            };
            output_path.set_last_modified_time(millis)?;
        }
    }
    Ok(())
}
