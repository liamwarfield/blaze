//! Port of `CompressedFunction` and its subclasses (`GzFunction`, `XzFunction`, `ZstFunction`,
//! `Bz2Function`): single compressed files, not archives.
//!
//! It ignores the prefix and strip_components settings because compressed files cannot contain
//! directories.

use std::io::{self, BufReader, Read, Write as _};

use blaze_vfs::{Path, PathFragment};

use super::compressed_tar::{BUFFER_SIZE_PUB as BUFFER_SIZE, decompressor_stream};
use super::{Decompressor, DecompressorDescriptor};
use crate::errors::DecompressorError;

/// Port of Commons Compress `FileNameUtil.getUncompressedFileName`: the shortest matching
/// suffix (compared case-insensitively) is replaced.
pub fn file_name_util_uncompressed(file_name: &str, suffixes: &[(&str, &str)]) -> String {
    let lower = file_name.to_lowercase();
    let n = lower.len();
    let shortest = suffixes.iter().map(|(k, _)| k.len()).min().unwrap_or(0);
    let longest = suffixes.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    let mut i = shortest;
    while i <= longest && i < n {
        if lower.is_char_boundary(n - i)
            && let Some((_, replacement)) = suffixes.iter().find(|(k, _)| *k == &lower[n - i..])
            && file_name.is_char_boundary(n - i)
        {
            return format!("{}{}", &file_name[..n - i], replacement);
        }
        i += 1;
    }
    file_name.to_string()
}

/// Commons Compress `GzipUtils` suffix map.
const GZIP_SUFFIXES: &[(&str, &str)] = &[
    (".tgz", ".tar"),
    (".taz", ".tar"),
    (".svgz", ".svg"),
    (".cpgz", ".cpio"),
    (".wmz", ".wmf"),
    (".emz", ".emf"),
    (".gz", ""),
    (".z", ""),
    ("-gz", ""),
    ("-z", ""),
    ("_z", ""),
];

/// Commons Compress `XZUtils` suffix map.
const XZ_SUFFIXES: &[(&str, &str)] = &[(".txz", ".tar"), (".xz", ""), ("-xz", "")];

/// Commons Compress `BZip2Utils` suffix map.
const BZIP2_SUFFIXES: &[(&str, &str)] = &[
    (".tar.bz2", ".tar"),
    (".tbz2", ".tar"),
    (".tbz", ".tar"),
    (".bz2", ""),
    (".bz", ""),
];

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/ZstFunction.java:32 ZstFunction.fileNameUtil
const ZST_SUFFIXES: &[(&str, &str)] = &[(".zst", "")];

/// Reads the gzip header of the first member (Commons Compress parses it eagerly).
struct GzipMetaData {
    file_name: Option<String>,
    modification_time: i64,
}

fn read_gzip_metadata(archive: &Path) -> Result<GzipMetaData, io::Error> {
    let input = archive.get_input_stream()?;
    let mut d = flate2::read::GzDecoder::new(BufReader::new(input));
    // Force header parsing.
    let mut buf = [0u8; 1];
    let _ = d.read(&mut buf)?;
    let header = d
        .header()
        .ok_or_else(|| io::Error::other("Input is not in the .gz format"))?;
    Ok(GzipMetaData {
        // Commons Compress decodes the FNAME field as ISO-8859-1.
        file_name: header
            .filename()
            .map(|b| b.iter().map(|&c| c as char).collect()),
        modification_time: header.mtime() as i64,
    })
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/CompressedFunction.java:59 CompressedFunction.decompress
pub(crate) fn decompress(
    which: Decompressor,
    descriptor: &DecompressorDescriptor,
) -> Result<Path, DecompressorError> {
    let rename_files = &descriptor.rename_files;
    let gzip_meta = if which == Decompressor::GzFunction {
        Some(read_gzip_metadata(&descriptor.archive_path)?)
    } else {
        None
    };
    let input = descriptor.archive_path.get_input_stream()?;
    let mut decompressor_stream =
        decompressor_stream(which, BufReader::with_capacity(BUFFER_SIZE, input))?;
    let base_name = descriptor.archive_path.get_base_name();
    let mut entry_name = match which {
        // Java: com/google/devtools/build/lib/bazel/repository/decompressor/GzFunction.java:38 GzFunction.getUncompressedFileName
        Decompressor::GzFunction => match gzip_meta.as_ref().and_then(|m| m.file_name.clone()) {
            // filename should be the simple basename + ext, but convert to a PathFragment and
            // run getBaseName to ensure that any path separators and uplevel references are
            // dropped.
            Some(f) if !f.trim().is_empty() => PathFragment::create(&f).get_base_name().to_string(),
            _ => file_name_util_uncompressed(base_name, GZIP_SUFFIXES),
        },
        // Java: com/google/devtools/build/lib/bazel/repository/decompressor/XzFunction.java:46 XzFunction.getUncompressedFileName
        Decompressor::XzFunction => file_name_util_uncompressed(base_name, XZ_SUFFIXES),
        // Java: com/google/devtools/build/lib/bazel/repository/decompressor/ZstFunction.java:40 ZstFunction.getUncompressedFileName
        Decompressor::ZstFunction => file_name_util_uncompressed(base_name, ZST_SUFFIXES),
        // Java: com/google/devtools/build/lib/bazel/repository/decompressor/Bz2Function.java:35 Bz2Function.getUncompressedFileName
        Decompressor::Bz2Function => file_name_util_uncompressed(base_name, BZIP2_SUFFIXES),
        other => panic!("not a compressor format: {:?}", other),
    };
    if let Some(renamed) = rename_files.get(&entry_name) {
        entry_name = renamed.clone();
    }
    let file_path = descriptor.destination_path.get_relative_str(&entry_name);
    if let Some(parent) = file_path.get_parent_directory() {
        parent.create_directory_and_parents()?;
    }
    {
        let mut out = file_path.get_output_stream()?;
        io::copy(&mut decompressor_stream, &mut out)?;
        out.flush()?;
    }
    // Java: com/google/devtools/build/lib/bazel/repository/decompressor/GzFunction.java:48 GzFunction.setFileAttributes
    if let Some(meta) = gzip_meta {
        // Commons Compress reports the gzip MTIME in milliseconds (seconds * 1000) and Bazel
        // multiplies by 1000 again; GzFunctionTest.setLastModifiedTime pins this behaviour.
        file_path.set_last_modified_time(meta.modification_time.wrapping_mul(1000 * 1000))?;
    }
    Ok(descriptor.destination_path.clone())
}
