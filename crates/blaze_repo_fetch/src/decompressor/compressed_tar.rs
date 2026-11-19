//! Port of `CompressedTarFunction` and its subclasses (`TarFunction`, `TarGzFunction`,
//! `TarXzFunction`, `TarZstFunction`, `TarBz2Function`).
//!
//! PORT: Java reads entries with Apache Commons Compress `TarArchiveInputStream`; Blaze uses the
//! `tar` crate, which resolves GNU long names/links and PAX `path`/`linkpath` the same way.
//! Entry names are raw bytes in both (USTAR as Latin-1, PAX as UTF-8, then Bazel's internal
//! encoding), so the names that reach the file system agree.

use std::io::{self, BufReader, Read, Write as _};

use indexmap::IndexMap;

use blaze_util::string_encoding::{latin1_decode, latin1_encode_lossy};
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

/// The numeric header fields of a tar entry that `CompressedTarFunction` reads, parsed the way
/// Commons Compress parses them when it reads the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TarHeaderNumbers {
    /// `TarArchiveEntry.getMode()`.
    pub(crate) mode: i32,
    /// The header's modification time in seconds (before any PAX `mtime` is applied).
    pub(crate) mod_time_seconds: i64,
}

/// The `mode` and `mtime` parts of `TarArchiveEntry.parseTarHeaderUnwrapped`, with the wrapping
/// of `TarArchiveEntry.parseTarHeader`: a field that `TarUtils` rejects with
/// `IllegalArgumentException` fails the entry with `IOException("Corrupted TAR archive.")`.
///
/// PORT: Commons Compress parses these fields of every entry when `getNextTarEntry` reads the
/// header, so a corrupt field fails before anything of the entry is extracted; the caller does
/// the same. The `tar` crate's own accessors differ (they reject a blank field, stop at the first
/// NUL, trim any whitespace and accept a sign), so they are not used for these fields. Commons
/// Compress also parses `uid`, `gid` and `size` here; Blaze does not read `uid`/`gid`, and `size`
/// is parsed inside the `tar` crate.
// Java: org/apache/commons/compress/archivers/tar/TarArchiveEntry.java TarArchiveEntry.parseTarHeader (commons-compress 1.26.1, read from the class file; the sources are not in the pinned tree)
pub(crate) fn parse_tar_header_numbers(
    header: &tar::Header,
) -> Result<TarHeaderNumbers, IoException> {
    let old = header.as_old();
    // mode = (int) parseOctalOrBinary(header, offset, MODELEN, lenient);
    let mode = parse_octal_or_binary(&old.mode).ok_or_else(corrupted_tar_archive)? as i32;
    // mTime = TimeUtils.unixTimeToFileTime(parseOctalOrBinary(header, offset, MODTIMELEN, lenient));
    let mod_time_seconds = parse_octal_or_binary(&old.mtime).ok_or_else(corrupted_tar_archive)?;
    Ok(TarHeaderNumbers {
        mode,
        mod_time_seconds,
    })
}

/// `new IOException("Corrupted TAR archive.", illegalArgumentException)`.
// PORT: the cause (TarUtils' "Invalid byte ... at offset ..." text) is not kept; Bazel prints
// only the IOException's message.
fn corrupted_tar_archive() -> IoException {
    IoException::new("Corrupted TAR archive.")
}

/// Commons Compress `TarUtils.parseOctalOrBinary(buffer, offset, length)` over one whole header
/// field. `None` is Java's `IllegalArgumentException`.
// Java: org/apache/commons/compress/archivers/tar/TarUtils.java TarUtils.parseOctalOrBinary (commons-compress 1.26.1)
pub(crate) fn parse_octal_or_binary(buffer: &[u8]) -> Option<i64> {
    if buffer[0] & 0x80 == 0 {
        return parse_octal(buffer);
    }
    let negative = buffer[0] == 0xff;
    if buffer.len() < 9 {
        return parse_binary_long(buffer, negative);
    }
    parse_binary_big_integer(buffer, negative)
}

/// Commons Compress `TarUtils.parseOctal(buffer, offset, length)` over one whole header field:
/// a field whose first byte is NUL is 0; leading spaces are skipped; all trailing NULs and spaces
/// are trimmed; every remaining byte must be an octal digit. `None` is Java's
/// `IllegalArgumentException`.
// Java: org/apache/commons/compress/archivers/tar/TarUtils.java TarUtils.parseOctal (commons-compress 1.26.1)
pub(crate) fn parse_octal(buffer: &[u8]) -> Option<i64> {
    let length = buffer.len();
    let mut result: i64 = 0;
    let mut end = length;
    let mut start = 0;

    if length < 2 {
        // "Length " + length + " must be at least 2"
        return None;
    }

    if buffer[start] == 0 {
        return Some(0);
    }

    // Skip leading spaces
    while start < end {
        if buffer[start] != b' ' {
            break;
        }
        start += 1;
    }

    // Trim all trailing NULs and spaces.
    // The ustar and POSIX tar specs require a trailing NUL or
    // space but some implementations use the extra digit for big
    // sizes/uids/gids ...
    let mut trailer = buffer[end - 1];
    while start < end && (trailer == 0 || trailer == b' ') {
        end -= 1;
        trailer = buffer[end - 1];
    }

    while start < end {
        let current_byte = buffer[start];
        if !(b'0'..=b'7').contains(&current_byte) {
            // exceptionMessage(buffer, offset, length, start, currentByte)
            return None;
        }
        result = (result << 3) + (current_byte - b'0') as i64; // convert from ASCII
        start += 1;
    }

    Some(result)
}

/// Commons Compress `TarUtils.parseBinaryLong` (base-256 fields shorter than 9 bytes).
// Java: org/apache/commons/compress/archivers/tar/TarUtils.java TarUtils.parseBinaryLong (commons-compress 1.26.1)
fn parse_binary_long(buffer: &[u8], negative: bool) -> Option<i64> {
    let length = buffer.len();
    if length >= 9 {
        // "At offset ..., ... byte binary number exceeds maximum signed long value"
        return None;
    }
    let mut val: i64 = 0;
    for b in &buffer[1..] {
        val = (val << 8) + (*b as i64);
    }
    if negative {
        // 2's complement
        val -= 1;
        // (long) Math.pow(2.0, (length - 1) * 8.0) - 1
        val ^= (1i64 << ((length - 1) * 8)) - 1;
    }
    Some(if negative { -val } else { val })
}

/// Commons Compress `TarUtils.parseBinaryBigInteger` (base-256 fields of 9 bytes or more).
// Java: org/apache/commons/compress/archivers/tar/TarUtils.java TarUtils.parseBinaryBigInteger (commons-compress 1.26.1)
fn parse_binary_big_integer(buffer: &[u8], negative: bool) -> Option<i64> {
    let remainder = &buffer[1..];
    // PORT: `new BigInteger(remainder)` (big-endian two's complement) held in an i128; tar header
    // fields are at most 12 bytes.
    if remainder.len() > 16 {
        blaze_util::unported!("TarUtils.parseBinaryBigInteger: field longer than 17 bytes");
    }
    let mut val: i128 = if remainder.first().is_some_and(|b| b & 0x80 != 0) {
        -1
    } else {
        0
    };
    for b in remainder {
        val = (val << 8) | (*b as i128);
    }
    if negative {
        // 2's complement: val.add(BigInteger.valueOf(-1)).not()
        val = !val.wrapping_sub(1);
    }
    // BigInteger.bitLength()
    let bit_length = if val < 0 {
        128 - (!val).leading_zeros()
    } else {
        128 - val.leading_zeros()
    };
    if bit_length > 63 {
        // "At offset ..., ... byte binary number exceeds maximum signed long value"
        return None;
    }
    let long_value = val as i64;
    Some(if negative {
        long_value.wrapping_neg()
    } else {
        long_value
    })
}

/// Commons Compress `TarArchiveEntry.getLastModifiedDate().getTime()`: the PAX `mtime` (with its
/// fraction, floored to milliseconds) when present, else the header's seconds.
fn last_modified_millis<R: Read>(
    entry: &mut tar::Entry<'_, R>,
    header_mod_time_seconds: i64,
) -> Result<i64, IoException> {
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
    // TimeUtils.unixTimeToFileTime(seconds) is FileTime.from(seconds, TimeUnit.SECONDS), and
    // getModTime() is new Date(fileTime.toMillis()); TimeUnit.SECONDS.toMillis saturates.
    Ok(header_mod_time_seconds.saturating_mul(1000))
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
    // Commons Compress does not verify header checksums (see `tar_checksum`).
    let mut tar_stream = tar::Archive::new(super::tar_checksum::LenientChecksumReader::new(
        decompressor_stream,
    ));
    for entry in tar_stream.entries()? {
        let mut entry = entry?;
        // Commons Compress parses the numeric header fields when getNextTarEntry reads the
        // header (see parse_tar_header_numbers).
        let header_numbers = parse_tar_header_numbers(entry.header())?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_pax_global_extensions() {
            // PORT: Commons Compress applies global PAX headers to later entries and does not
            // return them; Blaze skips them (they carry no path in practice).
            continue;
        }
        let raw_name = entry.path_bytes().into_owned();
        // Java: toRawBytesString(entry.getName()): the raw header bytes, one char per byte.
        // PORT: Commons Compress decodes a PAX `path` record as UTF-8 and Java re-encodes it
        // with `unicodeToInternal`, which replaces malformed UTF-8 in such a record; the `tar`
        // crate hands over the record's raw bytes.
        let mut entry_name = latin1_decode(&raw_name).into_owned();
        if let Some(renamed) = rename_files.get(&entry_name) {
            entry_name = renamed.clone();
        }
        // Java: entryName.getBytes(ISO_8859_1)
        let entry_path =
            StripPrefixedPath::maybe_deprefix(&latin1_encode_lossy(&entry_name), prefix);
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
            // Java: filePath.chmod(entry.getMode() | 0400), in finish_file.
            let mode = header_numbers.mode as u32;
            let mtime = last_modified_millis(&mut entry, header_numbers.mod_time_seconds)?;
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

#[cfg(test)]
mod tests {
    //! Expected values follow Commons Compress 1.26.1 `TarUtils` (see `parse_octal`).
    use super::{parse_octal, parse_octal_or_binary};

    #[test]
    fn parse_octal_digits() {
        assert_eq!(parse_octal(b"0000644\0"), Some(0o644));
        assert_eq!(parse_octal(b"0000644 "), Some(0o644));
        assert_eq!(parse_octal(b"  644 \0\0"), Some(0o644));
        assert_eq!(parse_octal(b"7777777\0"), Some(0o7777777));
        // No trailer: the extra digit is used.
        assert_eq!(parse_octal(b"77777777"), Some(0o77777777));
        assert_eq!(parse_octal(b"00000000012\0"), Some(10));
    }

    #[test]
    fn parse_octal_blank_field_is_zero() {
        assert_eq!(parse_octal(&[0u8; 8]), Some(0));
        assert_eq!(parse_octal(b"        "), Some(0));
        assert_eq!(parse_octal(b"       \0"), Some(0));
        assert_eq!(parse_octal(b" \0\0\0\0\0\0\0"), Some(0));
        // A leading NUL ends the field whatever follows.
        assert_eq!(parse_octal(b"\x00000644"), Some(0));
        assert_eq!(parse_octal(&[0u8; 12]), Some(0));
    }

    #[test]
    fn parse_octal_invalid_byte_is_illegal_argument() {
        assert_eq!(parse_octal(b"0000648\0"), None);
        assert_eq!(parse_octal(b"00006x4\0"), None);
        assert_eq!(parse_octal(b"644\0 755"), None);
        assert_eq!(parse_octal(b"+000644\0"), None);
        assert_eq!(parse_octal(b"-000644\0"), None);
        assert_eq!(parse_octal(b"\t00644\0\0"), None);
        assert_eq!(parse_octal(b"0"), None); // "Length 1 must be at least 2"
    }

    #[test]
    fn parse_octal_or_binary_base_256() {
        // Shorter than 9 bytes: parseBinaryLong.
        assert_eq!(
            parse_octal_or_binary(&[0x80, 0, 0, 0, 0, 0, 0x01, 0x00]),
            Some(256)
        );
        assert_eq!(parse_octal_or_binary(&[0xff; 8]), Some(-1));
        assert_eq!(
            parse_octal_or_binary(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]),
            Some(-256)
        );
        // 9 bytes or more: parseBinaryBigInteger.
        assert_eq!(
            parse_octal_or_binary(&[0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x00]),
            Some(256)
        );
        assert_eq!(parse_octal_or_binary(&[0xff; 12]), Some(-1));
        // "... byte binary number exceeds maximum signed long value"
        assert_eq!(
            parse_octal_or_binary(&[
                0x80, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff
            ]),
            None
        );
        assert_eq!(parse_octal_or_binary(b"0000644\0"), Some(0o644));
    }
}
