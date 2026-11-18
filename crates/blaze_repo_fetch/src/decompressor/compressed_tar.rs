//! Port of `CompressedTarFunction` and its subclasses (`TarFunction`, `TarGzFunction`,
//! `TarXzFunction`, `TarZstFunction`, `TarBz2Function`).
//!
//! PORT: Java reads entries with Apache Commons Compress `TarArchiveInputStream`; Blaze uses the
//! `tar` crate, which resolves GNU long names/links and PAX `path`/`linkpath` the same way.
//! Entry names are raw bytes in both (USTAR as Latin-1, PAX as UTF-8, then Bazel's internal
//! encoding), so the names that reach the file system agree.

use std::io::{self, BufReader, Read, Write as _};

use indexmap::IndexMap;

use blaze_vfs::{Path, PathFragment};

use super::{
    Decompressor, DecompressorDescriptor, StripPrefixedPath, could_not_find_prefix_exception,
    finish_file, hash_set_prefixes, maybe_make_prefix_suggestion,
};
use crate::errors::{DecompressorError, IoException};
use crate::fs_utils;

const BUFFER_SIZE: usize = 32 * 1024;

/// `getDecompressorStream` of each subclass.
pub(crate) const BUFFER_SIZE_PUB: usize = BUFFER_SIZE;

pub(crate) fn decompressor_stream(
    which: Decompressor,
    compressed: BufReader<Box<dyn Read + Send>>,
) -> Result<Box<dyn Read>, IoException> {
    Ok(match which {
        // Java: com/google/devtools/build/lib/bazel/repository/decompressor/TarFunction.java:27 TarFunction.getDecompressorStream
        Decompressor::TarFunction => Box::new(compressed),
        // Java: com/google/devtools/build/lib/bazel/repository/decompressor/TarGzFunction.java:31 TarGzFunction.getDecompressorStream
        Decompressor::TarGzFunction | Decompressor::GzFunction => {
            Box::new(flate2::bufread::MultiGzDecoder::new(compressed))
        }
        // Java: com/google/devtools/build/lib/bazel/repository/decompressor/TarXzFunction.java:31 TarXzFunction.getDecompressorStream
        Decompressor::TarXzFunction | Decompressor::XzFunction => {
            Box::new(xz2::bufread::XzDecoder::new_multi_decoder(compressed))
        }
        // Java: com/google/devtools/build/lib/bazel/repository/decompressor/TarZstFunction.java:30 TarZstFunction.getDecompressorStream
        Decompressor::TarZstFunction | Decompressor::ZstFunction => {
            let mut d = zstd::stream::read::Decoder::with_buffer(compressed)?;
            // Zstd.windowLogMax() on 64-bit platforms.
            d.window_log_max(31)?;
            Box::new(d)
        }
        // Java: com/google/devtools/build/lib/bazel/repository/decompressor/TarBz2Function.java:31 TarBz2Function.getDecompressorStream
        Decompressor::TarBz2Function | Decompressor::Bz2Function => {
            Box::new(bzip2::bufread::MultiBzDecoder::new(compressed))
        }
        other => panic!("not a compressed format: {:?}", other),
    })
}

/// Commons Compress `TarArchiveEntry.getLastModifiedDate().getTime()`: the PAX `mtime` (with its
/// fraction, floored to milliseconds) when present, else the header's seconds.
fn last_modified_millis<R: Read>(entry: &mut tar::Entry<'_, R>) -> Result<i64, IoException> {
    if let Some(exts) = entry.pax_extensions()? {
        for ext in exts {
            let ext = ext?;
            if ext.key_bytes() == b"mtime"
                && let Ok(v) = ext.value()
                && let Some(ms) = parse_pax_time_millis(v)
            {
                return Ok(ms);
            }
        }
    }
    let header = entry.header();
    // Commons Compress TarUtils.parseOctal: a field starting with NUL, or holding only spaces and
    // NULs, is 0 (old crates.io archives leave mtime blank); the tar crate rejects those.
    if commons_compress_octal_is_zero(&header.as_old().mtime) {
        return Ok(0);
    }
    Ok((header.mtime()? as i64).wrapping_mul(1000))
}

/// Commons Compress `TarUtils.parseOctal` returns 0 for a field whose first byte is NUL or which
/// is empty after skipping leading spaces and trimming trailing NULs and spaces (binary fields,
/// first byte >= 0x80, are parsed elsewhere).
// Java: org/apache/commons/compress/archivers/tar/TarUtils.java parseOctal (commons-compress 1.x)
fn commons_compress_octal_is_zero(field: &[u8]) -> bool {
    if field.is_empty() || field[0] == 0 {
        return true;
    }
    if field[0] & 0x80 != 0 {
        return false;
    }
    let mut start = 0;
    let mut end = field.len();
    while start < end && field[start] == b' ' {
        start += 1;
    }
    while start < end && (field[end - 1] == 0 || field[end - 1] == b' ') {
        end -= 1;
    }
    start == end
}

fn parse_pax_time_millis(v: &str) -> Option<i64> {
    let (int_part, frac) = match v.split_once('.') {
        Some((a, b)) => (a, b),
        None => (v, ""),
    };
    let negative = int_part.starts_with('-');
    let secs: i64 = int_part.parse().ok()?;
    let mut frac_ms: i64 = 0;
    for (i, c) in frac.chars().take(3).enumerate() {
        frac_ms += (c.to_digit(10)? as i64) * 10i64.pow(2 - i as u32);
    }
    let has_more = frac.chars().skip(3).any(|c| c != '0');
    Some(if negative {
        // floor for negative times
        secs * 1000 - frac_ms - if has_more { 1 } else { 0 }
    } else {
        secs * 1000 + frac_ms
    })
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/CompressedTarFunction.java:59 CompressedTarFunction.decompress
pub(crate) fn decompress(
    which: Decompressor,
    descriptor: &DecompressorDescriptor,
) -> Result<Path, DecompressorError> {
    let prefix = descriptor.prefix.as_deref();
    let rename_files = &descriptor.rename_files;
    let mut found_prefix = false;
    let mut available_prefixes: Vec<String> = Vec::new();
    // Store link, target info of symlinks, we create them after regular files are extracted.
    let mut symlinks: IndexMap<Path, PathFragment> = IndexMap::new();

    let input = descriptor.archive_path.get_input_stream()?;
    let compressed = BufReader::with_capacity(BUFFER_SIZE, input);
    let decompressor_stream = decompressor_stream(which, compressed)?;
    let mut tar_stream = tar::Archive::new(decompressor_stream);
    for entry in tar_stream.entries()? {
        let mut entry = entry?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_pax_global_extensions() {
            // PORT: Commons Compress applies global PAX headers to later entries and does not
            // return them; Blaze skips them (they carry no path in practice).
            continue;
        }
        let raw_name = entry.path_bytes().into_owned();
        let mut entry_name = String::from_utf8_lossy(&raw_name).into_owned();
        if let Some(renamed) = rename_files.get(&entry_name) {
            entry_name = renamed.clone();
        }
        let entry_path = StripPrefixedPath::maybe_deprefix(entry_name.as_bytes(), prefix);
        found_prefix = found_prefix || entry_path.found_prefix();

        if prefix.is_some()
            && !found_prefix
            && let Some(s) = maybe_make_prefix_suggestion(entry_path.get_path_fragment())
        {
            available_prefixes.push(s);
        }

        if entry_path.skip() {
            continue;
        }

        let stripped_relative_path = entry_path
            .get_path_fragment()
            .strip_components(descriptor.strip_components);
        if stripped_relative_path.is_empty() {
            continue;
        }
        let file_path = descriptor
            .destination_path
            .get_relative(&stripped_relative_path);
        if let Some(parent) = file_path.get_parent_directory() {
            parent.create_directory_and_parents()?;
        }
        // Commons Compress: isDirectory() is also true for a name ending in '/'.
        let is_directory = entry_type.is_dir() || entry_name.ends_with('/');
        if is_directory {
            file_path.create_directory_and_parents()?;
        } else if entry_type.is_symlink() || entry_type.is_hard_link() {
            let link_name = entry
                .link_name_bytes()
                .map(|c| c.into_owned())
                .unwrap_or_default();
            let target_name = StripPrefixedPath::maybe_deprefix_symlink(
                &link_name,
                prefix,
                &descriptor.destination_path,
            );
            if entry_type.is_symlink() {
                symlinks.insert(file_path, target_name);
            } else {
                let target_path = descriptor.destination_path.get_relative(&target_name);
                if file_path == target_path {
                    // The behavior here is semantically different, depending on whether the
                    // underlying filesystem is case-sensitive or case-insensitive. However, it
                    // is effectively the same: we drop the link entry.
                } else {
                    if file_path.exists() {
                        file_path.delete()?;
                    }
                    fs_utils::create_hard_link(&file_path, &target_path)?;
                }
            }
        } else {
            {
                let mut out = file_path.get_output_stream()?;
                io::copy(&mut entry, &mut out)?;
                out.flush()?;
            }
            let mode = entry.header().mode()?;
            let mtime = last_modified_millis(&mut entry)?;
            // This can only be done on real files, not links, or it will skip the reader to
            // the next "real" file to try to find the mod time info.
            finish_file(&file_path, mode, Some(mtime))?;
        }
    }

    for (link_path, target) in &symlinks {
        if link_path.exists() {
            link_path.delete()?;
        }
        fs_utils::ensure_symbolic_link(link_path, target)?;
    }

    if let Some(prefix) = prefix
        && !found_prefix
    {
        return Err(could_not_find_prefix_exception(
            prefix,
            &hash_set_prefixes(available_prefixes),
        )
        .into());
    }

    Ok(descriptor.destination_path.clone())
}

/// `strerror` text without Rust's " (os error N)" suffix.
pub(crate) fn strerror(e: &io::Error) -> String {
    let s = e.to_string();
    match s.find(" (os error") {
        Some(i) => s[..i].to_string(),
        None => s,
    }
}
