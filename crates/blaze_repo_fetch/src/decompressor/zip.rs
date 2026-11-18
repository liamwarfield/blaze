//! Port of `ZipDecompressor`, with a port of the reading half of Bazel's own zip library
//! (`com.google.devtools.build.zip.ZipReader`, from `src/java_tools/singlejar`).

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write as _};

use indexmap::IndexMap;

use blaze_util::check_state;
use blaze_vfs::{Path, PathFragment};

use super::strip_prefixed_path::create_path_fragment;
use super::{
    DecompressorDescriptor, StripPrefixedPath, could_not_find_prefix_exception, hash_set_prefixes,
    maybe_make_prefix_suggestion,
};
use crate::errors::{DecompressorError, IoException, IoExceptionKind};
use crate::fs_utils;

const MAX_PATH_LENGTH: u64 = 256;
const S_IFDIR: u32 = 0o40000;
const S_IFREG: u32 = 0o100000;
const S_IFLNK: u32 = 0o120000;
const EXECUTABLE_MASK: u32 = 0o755;

// source: https://docs.microsoft.com/en-us/windows/win32/fileio/file-attribute-constants
pub const WINDOWS_FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
pub const WINDOWS_FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
pub const WINDOWS_FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

fn zip_exception(message: String) -> IoException {
    IoException::of(IoExceptionKind::ZipException, message)
}

/// A central directory entry (`ZipFileEntry`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZipFileEntry {
    pub name: String,
    pub method: u16,
    /// Milliseconds since the epoch (DOS time interpreted in the local time zone).
    pub time: i64,
    pub compressed_size: u64,
    pub size: u64,
    pub external_attributes: u32,
    pub local_header_offset: u64,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn u64_at(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}

/// `Calendar` (default time zone) fields to epoch millis, via `mktime`.
fn local_time_millis(year: i32, month0: i32, day: i32, hour: i32, min: i32, sec: i32) -> i64 {
    // SAFETY: tm is plain-old-data; mktime only reads/normalizes the struct we own.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_year = year - 1900;
        tm.tm_mon = month0;
        tm.tm_mday = day;
        tm.tm_hour = hour;
        tm.tm_min = min;
        tm.tm_sec = sec;
        tm.tm_isdst = -1;
        libc::mktime(&mut tm) as i64 * 1000
    }
}

/// `ZipUtil.dosToUnixTime`.
// Java: src/java_tools/singlejar/java/com/google/devtools/build/zip/ZipUtil.java:183 ZipUtil.dosToUnixTime
fn dos_to_unix_time(timestamp: u32) -> i64 {
    let t = timestamp as i32;
    // Calendar.set(DAY_OF_MONTH, 0) is lenient; mktime normalizes the same way.
    local_time_millis(
        ((t >> 25) & 0x7f) + 1980,
        ((t >> 21) & 0x0f) - 1,
        (t >> 16) & 0x1f,
        (t >> 11) & 0x1f,
        (t >> 5) & 0x3f,
        (t & 0x1f) * 2,
    )
}

/// `ZipUtil.isValidInDos`.
fn is_valid_in_dos(time: i64) -> bool {
    let dos_epoch = local_time_millis(1980, 0, 1, 0, 0, 0);
    let max_dos_date = local_time_millis(2107, 11, 31, 23, 59, 59);
    time >= dos_epoch && time <= max_dos_date
}

/// Port of `ZipReader` (read side only).
pub struct ZipReader {
    file_name: String,
    file: File,
    entries: IndexMap<String, ZipFileEntry>,
}

impl ZipReader {
    // Java: src/java_tools/singlejar/java/com/google/devtools/build/zip/ZipReader.java:105 ZipReader.<init>
    pub fn open(path: &Path) -> Result<ZipReader, IoException> {
        let mut file = File::open(path.get_path_string()).map_err(|e| {
            IoException::of(
                if e.kind() == io::ErrorKind::NotFound {
                    IoExceptionKind::FileNotFoundException
                } else {
                    IoExceptionKind::IOException
                },
                format!("{} ({})", path, super::compressed_tar::strerror(&e)),
            )
        })?;
        let file_name = path.get_base_name().to_string();
        let len = file.metadata()?.len();
        let eocd = find_end_of_central_directory_record(&mut file, len, &file_name)?;
        let mut fixed = [0u8; 22];
        file.seek(SeekFrom::Start(eocd))?;
        file.read_exact(&mut fixed)?;
        let mut cd_size = u32_at(&fixed, 12) as u64;
        let mut cd_offset = u32_at(&fixed, 16) as u64;
        let mut expected_entries = u16_at(&fixed, 10) as u64;
        let maybe_zip64 = u16_at(&fixed, 4) == 0xffff
            || u16_at(&fixed, 6) == 0xffff
            || u16_at(&fixed, 8) == 0xffff
            || expected_entries == 0xffff
            || cd_size == 0xffff_ffff
            || cd_offset == 0xffff_ffff;
        let mut zip64 = false;
        if maybe_zip64 && eocd >= 20 {
            let mut loc = [0u8; 20];
            file.seek(SeekFrom::Start(eocd - 20))?;
            file.read_exact(&mut loc)?;
            if u32_at(&loc, 0) == 0x07064b50 {
                let z64_offset = u64_at(&loc, 8);
                let mut z = [0u8; 56];
                file.seek(SeekFrom::Start(z64_offset))?;
                if file.read_exact(&mut z).is_ok() && u32_at(&z, 0) == 0x06064b50 {
                    zip64 = true;
                    expected_entries = u64_at(&z, 32);
                    cd_size = u64_at(&z, 40);
                    cd_offset = u64_at(&z, 48);
                }
            }
        }
        let mut reader = ZipReader {
            file_name,
            file,
            entries: IndexMap::new(),
        };
        if zip64 {
            reader.read_central_directory_file_headers(
                Some(expected_entries),
                cd_offset,
                cd_size,
            )?;
        } else {
            // If not in Zip64 format, compute central directory offset by end of central
            // directory record offset and central directory size to allow reading large
            // non-compliant Zip32 directories.
            let computed = eocd.wrapping_sub(cd_size);
            if computed as u32 == cd_offset as u32 {
                reader.read_central_directory_file_headers(None, computed, cd_size)?;
            } else {
                reader.read_central_directory_file_headers(
                    Some(expected_entries),
                    cd_offset,
                    cd_size,
                )?;
            }
        }
        Ok(reader)
    }

    fn read_central_directory_file_headers(
        &mut self,
        count: Option<u64>,
        offset: u64,
        cd_size: u64,
    ) -> Result<(), IoException> {
        self.file.seek(SeekFrom::Start(offset))?;
        let mut r = BufReader::new(&self.file);
        let mut read: u64 = 0;
        let mut i: u64 = 0;
        loop {
            match count {
                Some(c) if i >= c => break,
                None if read >= cd_size => break,
                _ => {}
            }
            let entry = read_central_directory_file_header(&mut r, &mut read)?;
            // ZipFileData.addEntry: a LinkedHashMap keyed by name.
            self.entries.insert(entry.name.clone(), entry);
            i += 1;
        }
        Ok(())
    }

    /// `ZipReader.getFilename()`: the file's base name.
    pub fn get_filename(&self) -> &str {
        &self.file_name
    }

    pub fn entries(&self) -> impl Iterator<Item = &ZipFileEntry> {
        self.entries.values()
    }

    /// The decompressed content of an entry (`getInputStream`).
    // Java: src/java_tools/singlejar/java/com/google/devtools/build/zip/ZipEntryInputStream.java:42 ZipEntryInputStream.<init>
    pub fn get_input_stream(
        &self,
        entry: &ZipFileEntry,
    ) -> Result<Box<dyn Read + '_>, IoException> {
        let mut f = &self.file;
        f.seek(SeekFrom::Start(entry.local_header_offset))?;
        let mut header = [0u8; 30];
        let n = read_fully(&mut f, &mut header)?;
        if n < 4 || u32_at(&header, 0) != 0x04034b50 {
            return Err(zip_exception(format!(
                "The file '{}' is not a correctly formatted zip file: Expected a File Header at file offset {}, but was not present.",
                self.file_name, entry.local_header_offset
            )));
        }
        let skip = u16_at(&header, 26) as u64 + u16_at(&header, 28) as u64;
        f.seek(SeekFrom::Current(skip as i64))?;
        let reader = BufReader::new(f);
        if entry.method == 8 {
            Ok(Box::new(
                flate2::bufread::DeflateDecoder::new(reader).take(entry.size),
            ))
        } else {
            Ok(Box::new(reader.take(entry.size)))
        }
    }
}

fn read_fully<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        let k = r.read(&mut buf[n..])?;
        if k == 0 {
            break;
        }
        n += k;
    }
    Ok(n)
}

// Java: src/java_tools/singlejar/java/com/google/devtools/build/zip/CentralDirectoryFileHeader.java:58 CentralDirectoryFileHeader.read
fn read_central_directory_file_header<R: Read>(
    r: &mut R,
    read: &mut u64,
) -> Result<ZipFileEntry, IoException> {
    let mut fixed = [0u8; 46];
    if read_fully(r, &mut fixed)? != 46 {
        return Err(zip_exception(
            "Unexpected end of file while reading Central Directory File Header.".to_string(),
        ));
    }
    if u32_at(&fixed, 0) != 0x02014b50 {
        return Err(zip_exception(format!(
            "Malformed Central Directory File Header; does not start with {:08x}",
            0x02014b50
        )));
    }
    let name_len = u16_at(&fixed, 28) as usize;
    let extra_len = u16_at(&fixed, 30) as usize;
    let comment_len = u16_at(&fixed, 32) as usize;
    let mut name = vec![0u8; name_len];
    let mut extra = vec![0u8; extra_len];
    let mut comment = vec![0u8; comment_len];
    for b in [&mut name, &mut extra, &mut comment] {
        if !b.is_empty() && read_fully(r, b)? != b.len() {
            return Err(zip_exception(
                "Unexpected end of file while reading Central Directory File Header.".to_string(),
            ));
        }
    }
    *read += (46 + name_len + extra_len + comment_len) as u64;
    let mut csize = u32_at(&fixed, 20) as u64;
    let mut size = u32_at(&fixed, 24) as u64;
    let mut offset = u32_at(&fixed, 42) as u64;
    if csize == 0xffff_ffff || size == 0xffff_ffff || offset == 0xffff_ffff {
        // Find the Zip64 extra field (id 0x0001).
        let mut i = 0;
        while i + 4 <= extra.len() {
            let id = u16_at(&extra, i);
            let len = u16_at(&extra, i + 2) as usize;
            let data = &extra[i + 4..(i + 4 + len).min(extra.len())];
            if id == 0x0001 {
                let mut index = 0;
                if size == 0xffff_ffff && index + 8 <= data.len() {
                    size = u64_at(data, index);
                    index += 8;
                }
                if csize == 0xffff_ffff && index + 8 <= data.len() {
                    csize = u64_at(data, index);
                    index += 8;
                }
                if offset == 0xffff_ffff && index + 8 <= data.len() {
                    offset = u64_at(data, index);
                }
                break;
            }
            i += 4 + len;
        }
    }
    let time = dos_to_unix_time(u32_at(&fixed, 12));
    Ok(ZipFileEntry {
        // Java decodes with UTF-8, replacing malformed input.
        // PORT: byte-strings: nothing is kept as raw bytes here in Java either
        // (`new String(name, charset)` in CentralDirectoryFileHeader.read replaces malformed
        // input); ZipDecompressor then passes `entry.getName().getBytes(UTF_8)` on.
        name: String::from_utf8_lossy(&name).into_owned(),
        method: u16_at(&fixed, 10),
        time: if is_valid_in_dos(time) {
            time
        } else {
            local_time_millis(1980, 0, 1, 0, 0, 0)
        },
        compressed_size: csize,
        size,
        external_attributes: u32_at(&fixed, 38),
        local_header_offset: offset,
    })
}

/// Finds the end of central directory record whose comment length matches the file end.
// Java: src/java_tools/singlejar/java/com/google/devtools/build/zip/ZipReader.java:221 ZipReader.findEndOfCentralDirectoryRecord
fn find_end_of_central_directory_record(
    file: &mut File,
    len: u64,
    file_name: &str,
) -> Result<u64, IoException> {
    let malformed = || {
        zip_exception(format!(
            "Zip file '{}' is malformed. It does not contain an end of central directory record.",
            file_name
        ))
    };
    if len < 22 {
        return Err(malformed());
    }
    // PORT: Java scans backwards in 64-byte windows; reading the (at most 64 KiB + 22 byte)
    // tail at once and scanning it backwards finds the same record.
    let tail_len = len.min(0xffff + 22 + 64);
    let start = len - tail_len;
    let mut tail = vec![0u8; tail_len as usize];
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(&mut tail)?;
    let sig = 0x06054b50u32.to_le_bytes();
    let mut i = tail.len() as i64 - 22;
    while i >= 0 {
        let p = i as usize;
        if tail[p..p + 4] == sig {
            let comment_length = u16_at(&tail, p + 20) as u64;
            let eocd_size = tail.len() as u64 - p as u64;
            if comment_length == eocd_size - 22 {
                return Ok(start + p as u64);
            }
        }
        i -= 1;
    }
    Err(malformed())
}

/// This unzips the zip file to the destination directory.
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/ZipDecompressor.java:78 ZipDecompressor.decompress
pub(crate) fn decompress(descriptor: &DecompressorDescriptor) -> Result<Path, DecompressorError> {
    let destination_directory = &descriptor.destination_path;
    let prefix = descriptor.prefix.as_deref();
    let rename_files = &descriptor.rename_files;
    let mut found_prefix = false;
    // Store link, target info of symlinks, we create them after regular files are extracted.
    let mut symlinks: IndexMap<Path, PathFragment> = IndexMap::new();

    {
        let reader = ZipReader::open(&descriptor.archive_path)?;
        for entry in reader.entries() {
            let mut entry_name = entry.name.clone();
            if let Some(r) = rename_files.get(&entry_name) {
                entry_name = r.clone();
            }
            let entry_path = StripPrefixedPath::maybe_deprefix(entry_name.as_bytes(), prefix);
            found_prefix = found_prefix || entry_path.found_prefix();
            if entry_path.skip() {
                continue;
            }
            let path_fragment = entry_path
                .get_path_fragment()
                .strip_components(descriptor.strip_components);
            if path_fragment.is_empty() {
                continue;
            }
            extract_zip_entry(
                &reader,
                entry,
                destination_directory,
                &path_fragment,
                prefix,
                &mut symlinks,
            )?;
        }

        if let Some(prefix) = prefix
            && !found_prefix
        {
            let mut prefixes = Vec::new();
            for entry in reader.entries() {
                let entry_path = StripPrefixedPath::maybe_deprefix(entry.name.as_bytes(), None);
                if let Some(s) = maybe_make_prefix_suggestion(entry_path.get_path_fragment()) {
                    prefixes.push(s);
                }
            }
            return Err(
                could_not_find_prefix_exception(prefix, &hash_set_prefixes(prefixes)).into(),
            );
        }
    }

    for (link, target) in &symlinks {
        fs_utils::ensure_symbolic_link(link, target)?;
    }
    Ok(destination_directory.clone())
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/ZipDecompressor.java:128 ZipDecompressor.extractZipEntry
fn extract_zip_entry(
    reader: &ZipReader,
    entry: &ZipFileEntry,
    destination_directory: &Path,
    stripped_relative_path: &PathFragment,
    prefix: Option<&str>,
    symlinks: &mut IndexMap<Path, PathFragment>,
) -> Result<(), IoException> {
    if stripped_relative_path.is_absolute() {
        return Err(IoException::new(format!(
            "Failed to extract {}, zipped paths cannot be absolute",
            stripped_relative_path
        )));
    }
    let output_path = destination_directory.get_relative(stripped_relative_path);
    let permissions = get_permissions(entry.external_attributes, &entry.name)?;
    if let Some(parent) = output_path.get_parent_directory() {
        parent.create_directory_and_parents()?;
    }
    let is_directory = (permissions & S_IFDIR) == S_IFDIR;
    let is_symlink = (permissions & S_IFLNK) == S_IFLNK;
    if is_directory {
        output_path.create_directory_and_parents()?;
    } else if is_symlink {
        check_state!(entry.size < MAX_PATH_LENGTH);
        let mut buffer = vec![0u8; entry.size as usize];
        // For symlinks, the "compressed data" is actually the target name.
        let read = read_fully(&mut reader.get_input_stream(entry)?, &mut buffer)?;
        check_state!(read == buffer.len());

        let target = create_path_fragment(&buffer);
        if target.contains_uplevel_references() {
            let points_to = stripped_relative_path
                .get_parent_directory()
                .unwrap_or_else(PathFragment::empty_fragment)
                .get_relative(&target);
            if points_to.contains_uplevel_references() {
                return Err(IoException::new(format!(
                    "Zip entries cannot refer to files outside of their directory: {} has a symlink {} pointing to {}",
                    reader.get_filename(),
                    stripped_relative_path,
                    // PORT: byte-strings: Java prints `new String(buffer, UTF_8)` here (lossy
                    // too); the raw bytes of the target are kept only for the symlink itself
                    // (create_path_fragment).
                    String::from_utf8_lossy(&buffer)
                )));
            }
        }
        symlinks.insert(
            output_path,
            StripPrefixedPath::maybe_deprefix_symlink(&buffer, prefix, destination_directory),
        );
    } else {
        {
            let mut input = reader.get_input_stream(entry)?;
            let mut output = output_path.get_output_stream()?;
            io::copy(&mut input, &mut output)?;
            output.flush()?;
        }
        // Ensure that all files are at least user-readable. Some archives contain files that
        // are not, but many other tools are working around this and thus mask these issues.
        output_path.chmod(permissions | 0o400)?;
        output_path.set_last_modified_time(entry.time)?;
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/ZipDecompressor.java:186 ZipDecompressor.getPermissions
pub fn get_permissions(permissions: u32, path: &str) -> Result<u32, IoException> {
    // Sometimes zip files list directories as being "regular" executable files (i.e., 0100755).
    // I'm looking at you, Go AppEngine SDK 1.9.37 (see #1263 for details).
    if path.ends_with('/') {
        return Ok(S_IFDIR | EXECUTABLE_MASK);
    }

    // Posix permissions are in the high-order 2 bytes of the external attributes. After this
    // operation, permissions holds 0100755 (or 040755 for directories).
    let shifted_permissions = permissions >> 16;
    if shifted_permissions != 0 {
        return Ok(shifted_permissions);
    }

    // If this was zipped up on FAT, it won't have posix permissions set. Instead, this
    // checks if extra attributes is set to 0 for files.
    let windows_permission = permissions & 0xff;
    if (windows_permission & WINDOWS_FILE_ATTRIBUTE_DIRECTORY) == WINDOWS_FILE_ATTRIBUTE_DIRECTORY {
        // Directory.
        Ok(S_IFDIR | EXECUTABLE_MASK)
    } else if permissions == 0
        || (windows_permission & WINDOWS_FILE_ATTRIBUTE_ARCHIVE) == WINDOWS_FILE_ATTRIBUTE_ARCHIVE
        || (windows_permission & WINDOWS_FILE_ATTRIBUTE_NORMAL) == WINDOWS_FILE_ATTRIBUTE_NORMAL
    {
        // File.
        Ok(S_IFREG | EXECUTABLE_MASK)
    } else {
        // No idea.
        Err(IoException::new(format!(
            "Unrecognized file mode for {}: 0x{:x}",
            path, permissions
        )))
    }
}
