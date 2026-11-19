//! Port of `ArFunction`, with a port of Commons Compress `ArArchiveInputStream` (GNU and BSD
//! long names). It ignores the prefix setting because these archives cannot contain
//! directories.

use std::io::{self, BufReader, Read, Write as _};

use blaze_vfs::Path;

use super::DecompressorDescriptor;
use super::finish_file;
use crate::errors::{DecompressorError, IoException, IoExceptionKind};
use crate::fs_utils;

const BUFFER_SIZE: usize = 32 * 1024;
const HEADER: &[u8] = b"!<arch>\n";
const TRAILER: &[u8] = b"`\n";
const NAME_LEN: usize = 16;
const LAST_MODIFIED_LEN: usize = 12;
const USER_ID_LEN: usize = 6;
const GROUP_ID_LEN: usize = 6;
const FILE_MODE_LEN: usize = 8;
const LENGTH_LEN: usize = 10;
const META_LEN: usize =
    NAME_LEN + LAST_MODIFIED_LEN + USER_ID_LEN + GROUP_ID_LEN + FILE_MODE_LEN + LENGTH_LEN;
const LAST_MODIFIED_OFFSET: usize = NAME_LEN;
const FILE_MODE_OFFSET: usize = NAME_LEN + LAST_MODIFIED_LEN + USER_ID_LEN + GROUP_ID_LEN;
const LENGTH_OFFSET: usize = FILE_MODE_OFFSET + FILE_MODE_LEN;

/// One `ArArchiveEntry`.
#[derive(Clone, Debug)]
pub struct ArArchiveEntry {
    pub name: String,
    pub length: u64,
    pub mode: u32,
    pub last_modified: i64,
}

/// Port of Commons Compress `ArArchiveInputStream`.
pub struct ArArchiveInputStream<R: Read> {
    input: R,
    offset: u64,
    entry_offset: u64,
    current_length: Option<u64>,
    namebuffer: Option<Vec<u8>>,
}

/// Commons Compress `ArchiveUtils.toAsciiString`: `new String(bytes, US_ASCII)`, a Unicode
/// string in which every byte >= 0x80 is U+FFFD.
fn ascii(b: &[u8]) -> String {
    b.iter()
        .map(|&c| if c < 0x80 { c as char } else { '\u{fffd}' })
        .collect()
}

fn read_range<R: Read>(r: &mut R, len: usize) -> io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(len);
    r.take(len as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Java `String.trim()`: strips chars `<= ' '`.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

fn parse_long(s: &str, what: &str) -> Result<i64, IoException> {
    java_trim(s).parse::<i64>().map_err(|_| {
        IoException::new(format!(
            "Broken archive, unable to parse {} as a number",
            what
        ))
    })
}

impl<R: Read> ArArchiveInputStream<R> {
    pub fn new(input: R) -> Self {
        ArArchiveInputStream {
            input,
            offset: 0,
            entry_offset: 0,
            current_length: None,
            namebuffer: None,
        }
    }

    fn skip(&mut self, n: u64) -> io::Result<u64> {
        io::copy(&mut (&mut self.input).take(n), &mut io::sink())
    }

    /// `getNextArEntry`.
    pub fn get_next_ar_entry(&mut self) -> Result<Option<ArArchiveEntry>, IoException> {
        if let Some(len) = self.current_length.take() {
            let entry_end = self.entry_offset + len;
            let skipped = self.skip(entry_end - self.offset)?;
            self.offset += skipped;
        }
        if self.offset == 0 {
            let realized = read_range(&mut self.input, HEADER.len())?;
            self.offset += realized.len() as u64;
            if realized.len() != HEADER.len() {
                return Err(IoException::new(format!(
                    "Failed to read header. Occurred at byte: {}",
                    self.offset
                )));
            }
            if realized != HEADER {
                return Err(IoException::new(format!(
                    "Invalid header {}",
                    ascii(&realized)
                )));
            }
        }
        if !self.offset.is_multiple_of(2) {
            let mut b = [0u8; 1];
            if self.input.read(&mut b)? == 0 {
                // hit eof
                return Ok(None);
            }
            self.offset += 1;
        }
        let meta = read_range(&mut self.input, META_LEN)?;
        self.offset += meta.len() as u64;
        if meta.is_empty() {
            return Ok(None);
        }
        if meta.len() < META_LEN {
            return Err(IoException::new("Truncated ar archive"));
        }
        let realized = read_range(&mut self.input, TRAILER.len())?;
        self.offset += realized.len() as u64;
        if realized.len() != TRAILER.len() {
            return Err(IoException::new(format!(
                "Failed to read entry trailer. Occurred at byte: {}",
                self.offset
            )));
        }
        if realized != TRAILER {
            return Err(IoException::new(format!(
                "Invalid entry trailer. not read the content? Occurred at byte: {}",
                self.offset
            )));
        }
        self.entry_offset = self.offset;

        let mut temp = java_trim(&ascii(&meta[..NAME_LEN])).to_string();
        let length_str = ascii(&meta[LENGTH_OFFSET..LENGTH_OFFSET + LENGTH_LEN]);
        if temp == "//" {
            // GNU extended file names entry
            let bufflen = parse_long(&length_str, "GNU string table length field")? as usize;
            let nb = read_range(&mut self.input, bufflen)?;
            self.offset += nb.len() as u64;
            if nb.len() != bufflen {
                return Err(IoException::new(format!(
                    "Failed to read complete // record: expected={} read={}",
                    bufflen,
                    nb.len()
                )));
            }
            self.namebuffer = Some(nb);
            self.current_length = Some(bufflen as u64);
            return self.get_next_ar_entry();
        }
        let mut len = parse_long(&length_str, "ar_size field")?;
        if let Some(stripped) = temp.strip_suffix('/') {
            // GNU terminator
            temp = stripped.to_string();
        } else if temp.len() > 1
            && temp.starts_with('/')
            && temp[1..].chars().all(|c| c.is_ascii_digit())
        {
            let off: usize = temp[1..]
                .parse()
                .map_err(|_| IoException::new("Broken archive"))?;
            temp = self.get_extended_name(off)?;
        } else if let Some(n) = temp.strip_prefix("#1/")
            && !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
        {
            let name_len: usize = n.parse().map_err(|_| IoException::new("Broken archive"))?;
            let name = read_range(&mut self.input, name_len)?;
            self.offset += name.len() as u64;
            if name.len() != name_len {
                return Err(IoException::without_message(IoExceptionKind::EOFException));
            }
            temp = ascii(&name);
            // entry length contained the length of the file name in addition to the real
            // length of the entry.
            // Java: temp.length(), one char per byte.
            len -= name_len as i64;
            self.entry_offset += name_len as u64;
        }
        if len < 0 {
            return Err(IoException::new("broken archive, entry with negative size"));
        }
        let mode_str = ascii(&meta[FILE_MODE_OFFSET..FILE_MODE_OFFSET + FILE_MODE_LEN]);
        let mode = u32::from_str_radix(java_trim(&mode_str), 8).map_err(|_| {
            IoException::new("Broken archive, unable to parse entry metadata fields as numbers")
        })?;
        let last_modified = parse_long(
            &ascii(&meta[LAST_MODIFIED_OFFSET..LAST_MODIFIED_OFFSET + LAST_MODIFIED_LEN]),
            "entry metadata fields",
        )?;
        self.current_length = Some(len as u64);
        Ok(Some(ArArchiveEntry {
            name: temp,
            length: len as u64,
            mode,
            last_modified,
        }))
    }

    fn get_extended_name(&self, offset: usize) -> Result<String, IoException> {
        let Some(nb) = &self.namebuffer else {
            return Err(IoException::new(
                "Cannot process GNU long file name as no // record was found",
            ));
        };
        let mut i = offset;
        while i < nb.len() {
            if nb[i] == b'\n' || nb[i] == 0 {
                if i == 0 {
                    break;
                }
                if nb[i - 1] == b'/' {
                    i -= 1; // drop trailing /
                }
                if i > offset {
                    return Ok(ascii(&nb[offset..i]));
                }
                break;
            }
            i += 1;
        }
        Err(IoException::new(format!(
            "Failed to read entry: {}",
            offset
        )))
    }
}

impl<R: Read> Read for ArArchiveInputStream<R> {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        if b.is_empty() {
            return Ok(0);
        }
        let Some(len) = self.current_length else {
            panic!("IllegalStateException: No current ar entry");
        };
        let entry_end = self.entry_offset + len;
        if self.offset >= entry_end {
            return Ok(0);
        }
        let to_read = (b.len() as u64).min(entry_end - self.offset) as usize;
        let ret = self.input.read(&mut b[..to_read])?;
        self.offset += ret as u64;
        Ok(ret)
    }
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/ArFunction.java:41 ArFunction.decompress
pub(crate) fn decompress(descriptor: &DecompressorDescriptor) -> Result<Path, DecompressorError> {
    let rename_files = &descriptor.rename_files;
    let input = BufReader::with_capacity(BUFFER_SIZE, descriptor.archive_path.get_input_stream()?);
    let mut ar_stream = ArArchiveInputStream::new(input);
    while let Some(entry) = ar_stream.get_next_ar_entry()? {
        let entry_name = rename_files
            .get(&entry.name)
            .cloned()
            .unwrap_or(entry.name.clone());
        // `entry_name` is Unicode (or an internal string from `rename_files`); Java puts it into
        // the path unconverted.
        let file_path = descriptor
            .destination_path
            .get_relative_str(&fs_utils::unicode_path_as_seen_by_file_system(&entry_name));
        if let Some(parent) = file_path.get_parent_directory() {
            parent.create_directory_and_parents()?;
        }
        // ar archives don't contain any directory information (isDirectory() is always
        // false), and they do not support symlinks.
        {
            let mut out = file_path.get_output_stream()?;
            io::copy(&mut ar_stream, &mut out)?;
            out.flush()?;
        }
        // entry.getLastModified() appears to be in seconds, so we need to convert it into
        // milliseconds for setLastModifiedTime
        finish_file(&file_path, entry.mode, Some(entry.last_modified * 1000))?;
    }
    Ok(descriptor.destination_path.clone())
}
