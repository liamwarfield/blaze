//! Port of `lib/util/MapCodec.java`, with the `java.io.DataInput`/`DataOutput` encodings it
//! relies on (big-endian, as `DataInputStream`/`DataOutputStream`).
//!
//! PORT: belongs to `lib/util`; it lives here (its only users are `PersistentMap` and the action
//! cache) until that package is ported.

use std::fmt;
use std::io::{self, Read};

use blaze_vfs::Path;

use super::record_output_stream::RecordOutputStream;

/// Port of `MapCodec.IncompatibleFormatException`: persisted data read back from disk is in an
/// incompatible format. Carried inside an [`io::Error`] (Java: a subclass of `IOException`); test
/// for it with [`is_incompatible_format_exception`].
// Java: com/google/devtools/build/lib/util/MapCodec.java:32 MapCodec.IncompatibleFormatException
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncompatibleFormatException {
    message: String,
}

impl IncompatibleFormatException {
    // Java: com/google/devtools/build/lib/util/MapCodec.java:33 MapCodec.IncompatibleFormatException.<init>
    pub fn new(message: impl Into<String>) -> IncompatibleFormatException {
        IncompatibleFormatException {
            message: message.into(),
        }
    }

    /// Wraps the exception in an [`io::Error`] (Java: it is an `IOException`).
    pub fn into_io_error(self) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, self)
    }
}

impl fmt::Display for IncompatibleFormatException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for IncompatibleFormatException {}

/// `e instanceof IncompatibleFormatException`.
pub fn is_incompatible_format_exception(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|inner| {
        inner
            .downcast_ref::<IncompatibleFormatException>()
            .is_some()
    })
}

fn eof() -> io::Error {
    // Java: java.io.EOFException (no message).
    io::Error::from(io::ErrorKind::UnexpectedEof)
}

/// `java.io.DataInputStream` over an in-memory byte array.
#[derive(Debug)]
pub struct DataInput {
    bytes: Vec<u8>,
    position: usize,
}

impl DataInput {
    pub fn new(bytes: Vec<u8>) -> DataInput {
        DataInput { bytes, position: 0 }
    }

    /// `available()`.
    pub fn available(&self) -> usize {
        self.bytes.len() - self.position
    }

    /// `readFully(byte[])`.
    pub fn read_fully(&mut self, dst: &mut [u8]) -> io::Result<()> {
        if dst.len() > self.available() {
            // DataInputStream consumes what is left before throwing.
            self.position = self.bytes.len();
            return Err(eof());
        }
        dst.copy_from_slice(&self.bytes[self.position..self.position + dst.len()]);
        self.position += dst.len();
        Ok(())
    }

    /// `readUnsignedByte()`.
    pub fn read_unsigned_byte(&mut self) -> io::Result<u8> {
        let mut b = [0u8; 1];
        self.read_fully(&mut b)?;
        Ok(b[0])
    }

    /// `readBoolean()`.
    pub fn read_boolean(&mut self) -> io::Result<bool> {
        Ok(self.read_unsigned_byte()? != 0)
    }

    /// `readInt()`.
    pub fn read_int(&mut self) -> io::Result<i32> {
        let mut b = [0u8; 4];
        self.read_fully(&mut b)?;
        Ok(i32::from_be_bytes(b))
    }

    /// `readLong()`.
    pub fn read_long(&mut self) -> io::Result<i64> {
        let mut b = [0u8; 8];
        self.read_fully(&mut b)?;
        Ok(i64::from_be_bytes(b))
    }

    /// `readUnsignedShort()`.
    pub fn read_unsigned_short(&mut self) -> io::Result<u16> {
        let mut b = [0u8; 2];
        self.read_fully(&mut b)?;
        Ok(u16::from_be_bytes(b))
    }

    /// `readUTF()`: a length-prefixed modified UTF-8 string.
    pub fn read_utf(&mut self) -> io::Result<String> {
        let utflen = self.read_unsigned_short()? as usize;
        let mut bytes = vec![0u8; utflen];
        self.read_fully(&mut bytes)?;
        let malformed = |count: usize| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("malformed input around byte {}", count),
            )
        };
        let mut chars: Vec<u16> = Vec::with_capacity(utflen);
        let mut count = 0;
        while count < utflen {
            let c = bytes[count] as u16;
            match c >> 4 {
                0..=7 => {
                    // 0xxxxxxx
                    count += 1;
                    chars.push(c);
                }
                12 | 13 => {
                    // 110x xxxx   10xx xxxx
                    count += 2;
                    if count > utflen {
                        return Err(malformed(count));
                    }
                    let char2 = bytes[count - 1] as u16;
                    if (char2 & 0xC0) != 0x80 {
                        return Err(malformed(count));
                    }
                    chars.push(((c & 0x1F) << 6) | (char2 & 0x3F));
                }
                14 => {
                    // 1110 xxxx  10xx xxxx  10xx xxxx
                    count += 3;
                    if count > utflen {
                        return Err(malformed(count));
                    }
                    let char2 = bytes[count - 2] as u16;
                    let char3 = bytes[count - 1] as u16;
                    if (char2 & 0xC0) != 0x80 || (char3 & 0xC0) != 0x80 {
                        return Err(malformed(count - 1));
                    }
                    chars.push(((c & 0x0F) << 12) | ((char2 & 0x3F) << 6) | (char3 & 0x3F));
                }
                _ => {
                    // 10xx xxxx,  1111 xxxx
                    return Err(malformed(count));
                }
            }
        }
        Ok(String::from_utf16_lossy(&chars))
    }
}

/// `java.io.DataOutputStream` over a [`RecordOutputStream`].
pub struct DataOutput<'a> {
    out: &'a mut RecordOutputStream,
}

impl<'a> DataOutput<'a> {
    pub fn new(out: &'a mut RecordOutputStream) -> DataOutput<'a> {
        DataOutput { out }
    }

    /// `write(byte[])`.
    pub fn write(&mut self, b: &[u8]) -> io::Result<()> {
        self.out.write_bytes(b)
    }

    /// `writeByte(int)`.
    pub fn write_byte(&mut self, v: i32) -> io::Result<()> {
        self.out.write_bytes(&[v as u8])
    }

    /// `writeBoolean(boolean)`.
    pub fn write_boolean(&mut self, v: bool) -> io::Result<()> {
        self.out.write_bytes(&[v as u8])
    }

    /// `writeInt(int)`.
    pub fn write_int(&mut self, v: i32) -> io::Result<()> {
        self.out.write_bytes(&v.to_be_bytes())
    }

    /// `writeLong(long)`.
    pub fn write_long(&mut self, v: i64) -> io::Result<()> {
        self.out.write_bytes(&v.to_be_bytes())
    }

    /// `writeUTF(String)`: a length-prefixed modified UTF-8 string.
    pub fn write_utf(&mut self, s: &str) -> io::Result<()> {
        let mut bytes = Vec::with_capacity(s.len());
        for c in s.encode_utf16() {
            if (0x0001..=0x007F).contains(&c) {
                bytes.push(c as u8);
            } else if c > 0x07FF {
                bytes.push((0xE0 | ((c >> 12) & 0x0F)) as u8);
                bytes.push((0x80 | ((c >> 6) & 0x3F)) as u8);
                bytes.push((0x80 | (c & 0x3F)) as u8);
            } else {
                bytes.push((0xC0 | ((c >> 6) & 0x1F)) as u8);
                bytes.push((0x80 | (c & 0x3F)) as u8);
            }
        }
        if bytes.len() > 65535 {
            return Err(io::Error::other(format!(
                "encoded string ({}...{}) too long: {} bytes",
                s.chars().take(8).collect::<String>(),
                s.chars()
                    .rev()
                    .take(8)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>(),
                bytes.len()
            )));
        }
        self.out.write_bytes(&(bytes.len() as u16).to_be_bytes())?;
        self.out.write_bytes(&bytes)
    }
}

/// Port of `MapCodec`: converts map entries between their in-memory and on-disk representations.
// Java: com/google/devtools/build/lib/util/MapCodec.java:30 MapCodec
pub trait MapCodec<K, V>: Send + Sync {
    /// Reads a key from a `DataInput`.
    // Java: com/google/devtools/build/lib/util/MapCodec.java:44 MapCodec.readKey
    fn read_key(&self, in_: &mut DataInput) -> io::Result<K>;

    /// Reads a value from a `DataInput`.
    // Java: com/google/devtools/build/lib/util/MapCodec.java:47 MapCodec.readValue
    fn read_value(&self, in_: &mut DataInput) -> io::Result<V>;

    /// Writes a key into a `DataOutput`.
    // Java: com/google/devtools/build/lib/util/MapCodec.java:50 MapCodec.writeKey
    fn write_key(&self, key: &K, out: &mut DataOutput<'_>) -> io::Result<()>;

    /// Writes a value into a `DataOutput`.
    // Java: com/google/devtools/build/lib/util/MapCodec.java:53 MapCodec.writeValue
    fn write_value(&self, value: &V, out: &mut DataOutput<'_>) -> io::Result<()>;
}

// Java: com/google/devtools/build/lib/util/MapCodec.java:38 MapCodec.MAGIC
const MAGIC: i64 = 0x20071105;
// Java: com/google/devtools/build/lib/util/MapCodec.java:39 MapCodec.MIN_MAPFILE_SIZE
const MIN_MAPFILE_SIZE: i64 = 16;
// Java: com/google/devtools/build/lib/util/MapCodec.java:40 MapCodec.MAX_ARRAY_SIZE
const MAX_ARRAY_SIZE: i64 = i32::MAX as i64 - 8;
// Java: com/google/devtools/build/lib/util/MapCodec.java:41 MapCodec.ENTRY_MAGIC
const ENTRY_MAGIC: u8 = 0xfe;

/// A key/value pair representing the presence or absence of a map entry (`value` is `None` for
/// an absent entry).
// Java: com/google/devtools/build/lib/util/MapCodec.java:61 MapCodec.Entry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapCodecEntry<K, V> {
    pub key: K,
    pub value: Option<V>,
}

/// Creates a new reader.
///
/// The file contents are eagerly read into memory, under the assumption that they will be
/// iterated to completion in short order.
///
/// Fails with an [`IncompatibleFormatException`] if the on-disk data is in an incompatible
/// format, or another I/O error if data corruption is detected or some other I/O error occurs.
// Java: com/google/devtools/build/lib/util/MapCodec.java:74 MapCodec.createReader
pub fn create_reader<'c, K, V>(
    codec: &'c dyn MapCodec<K, V>,
    path: &Path,
    version: i64,
) -> io::Result<MapCodecReader<'c, K, V>> {
    let size = path.get_file_size()?;
    if size < MIN_MAPFILE_SIZE {
        return Err(IncompatibleFormatException::new(format!(
            "{} is too short: {} bytes",
            path, size
        ))
        .into_io_error());
    }
    if size > MAX_ARRAY_SIZE {
        return Err(IncompatibleFormatException::new(format!(
            "{} is too long: {} bytes",
            path, size
        ))
        .into_io_error());
    }

    // Read the whole file upfront as a performance optimization (minimize syscalls).
    let mut bytes = Vec::new();
    {
        let mut in_ = path.get_input_stream()?;
        in_.read_to_end(&mut bytes)?;
    }

    let mut in_ = DataInput::new(bytes);

    if in_.read_long()? != MAGIC {
        // Not a PersistentMap.
        return Err(IncompatibleFormatException::new("Bad magic number").into_io_error());
    }
    let persisted_version = in_.read_long()?;
    if persisted_version != version {
        // Incompatible version.
        return Err(IncompatibleFormatException::new(format!(
            "Incompatible version: want {}, got {}",
            version, persisted_version
        ))
        .into_io_error());
    }

    Ok(MapCodecReader { codec, in_ })
}

/// Reads key/value pairs from a `DataInputStream`.
// Java: com/google/devtools/build/lib/util/MapCodec.java:106 MapCodec.Reader
pub struct MapCodecReader<'c, K, V> {
    codec: &'c dyn MapCodec<K, V>,
    in_: DataInput,
}

impl<K, V> MapCodecReader<'_, K, V> {
    /// Reads an entry; `Ok(None)` if there are no more entries.
    // Java: com/google/devtools/build/lib/util/MapCodec.java:125 MapCodec.Reader.readEntry
    pub fn read_entry(&mut self) -> io::Result<Option<MapCodecEntry<K, V>>> {
        if self.in_.available() == 0 {
            return Ok(None);
        }
        if self.in_.read_unsigned_byte()? != ENTRY_MAGIC {
            return Err(io::Error::other("Corrupted entry separator"));
        }
        let key = self.codec.read_key(&mut self.in_)?;
        let has_value = self.in_.read_boolean()?;
        let value = if has_value {
            Some(self.codec.read_value(&mut self.in_)?)
        } else {
            None
        };
        Ok(Some(MapCodecEntry { key, value }))
    }
}

/// Creates a new writer.
///
/// `overwrite`: whether to overwrite an existing file instead of appending to it.
// Java: com/google/devtools/build/lib/util/MapCodec.java:147 MapCodec.createWriter
pub fn create_writer<'c, K, V>(
    codec: &'c dyn MapCodec<K, V>,
    path: &Path,
    version: i32,
    overwrite: bool,
) -> io::Result<MapCodecWriter<'c, K, V>> {
    let append = !overwrite && path.exists();
    let mut record_out =
        RecordOutputStream::new(path.get_output_stream_with(append, /* internal= */ false)?);
    if !append {
        let mut data_out = DataOutput::new(&mut record_out);
        data_out.write_long(MAGIC)?;
        data_out.write_long(version as i64)?;
        record_out.finish_record();
    }
    Ok(MapCodecWriter { codec, record_out })
}

/// Writes key/value pairs to a `DataOutputStream` backed by a [`RecordOutputStream`].
///
/// In a best-effort attempt to prevent data corruption in the event of an abrupt exit, use a
/// [`RecordOutputStream`] instead of a `BufferedOutputStream` to ensure that only complete
/// records are ever written to the underlying unbuffered stream. While this can still be defeated
/// by partial writes, experiments suggest they're rather unlikely for small buffer sizes.
// Java: com/google/devtools/build/lib/util/MapCodec.java:168 MapCodec.Writer
pub struct MapCodecWriter<'c, K, V> {
    codec: &'c dyn MapCodec<K, V>,
    record_out: RecordOutputStream,
}

impl<K, V> MapCodecWriter<'_, K, V> {
    /// Flushes the writer, forcing any pending writes to be written to disk.
    // Java: com/google/devtools/build/lib/util/MapCodec.java:178 MapCodec.Writer.flush
    pub fn flush(&mut self) -> io::Result<()> {
        self.record_out.flush_records()
    }

    /// Closes the writer, releasing associated resources and rendering it unusable.
    // Java: com/google/devtools/build/lib/util/MapCodec.java:184 MapCodec.Writer.close
    pub fn close(mut self) -> io::Result<()> {
        self.record_out.close()
    }

    /// Writes a key/value pair; `None` writes a tombstone.
    // Java: com/google/devtools/build/lib/util/MapCodec.java:194 MapCodec.Writer.writeEntry
    pub fn write_entry(&mut self, key: &K, value: Option<&V>) -> io::Result<()> {
        let mut data_out = DataOutput::new(&mut self.record_out);
        data_out.write_byte(ENTRY_MAGIC as i32)?;
        self.codec.write_key(key, &mut data_out)?;
        let has_value = value.is_some();
        data_out.write_boolean(has_value)?;
        if let Some(value) = value {
            self.codec.write_value(value, &mut data_out)?;
        }
        self.record_out.finish_record();
        Ok(())
    }
}
