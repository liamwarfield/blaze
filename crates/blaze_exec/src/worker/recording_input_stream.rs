//! Port of `lib/worker/RecordingInputStream.java`, and [`PipeInputStream`], the worker process's
//! stdout as Java's `Process.getInputStream()` gives it (a stream with `available()`).

use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::Arc;

use parking_lot::{Condvar, Mutex};

/// An `InputStream`: `read` and `available()`.
pub trait InputStream: Send {
    /// `read(byte[], int, int)`: blocks until at least one byte is available; `Ok(0)` at the end
    /// of the stream (Java's `-1`) or for an empty `buf`.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    /// `available()`: the number of bytes that can be read without blocking.
    fn available(&mut self) -> io::Result<usize>;
}

/// The stdout of a worker process: a pipe read by a helper thread into a buffer, so that
/// `available()` reports what the process has written and not yet been read.
///
/// PORT: Java's process stream is a `BufferedInputStream` over the pipe's `FileInputStream`,
/// whose `available()` asks the kernel (`FIONREAD`). Blaze's subprocess layer hands out a plain
/// `Read`; the helper thread moves the bytes into this buffer as they arrive, which gives the same
/// answers.
pub struct PipeInputStream {
    shared: Arc<PipeShared>,
}

struct PipeShared {
    state: Mutex<PipeState>,
    cv: Condvar,
}

#[derive(Default)]
struct PipeState {
    buf: VecDeque<u8>,
    eof: bool,
    error: Option<(io::ErrorKind, String)>,
}

impl PipeInputStream {
    /// Starts reading `source` on a helper thread named `name`.
    pub fn new(mut source: Box<dyn Read + Send>, name: &str) -> PipeInputStream {
        let shared = Arc::new(PipeShared {
            state: Mutex::new(PipeState::default()),
            cv: Condvar::new(),
        });
        let pump = shared.clone();
        let spawned = std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || {
                let mut chunk = vec![0u8; 8192];
                loop {
                    let r = source.read(&mut chunk);
                    let mut st = pump.state.lock();
                    match r {
                        Ok(0) => {
                            st.eof = true;
                            pump.cv.notify_all();
                            return;
                        }
                        Ok(n) => st.buf.extend(&chunk[..n]),
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => {
                            st.error = Some((e.kind(), e.to_string()));
                            st.eof = true;
                            pump.cv.notify_all();
                            return;
                        }
                    }
                    pump.cv.notify_all();
                }
            });
        if spawned.is_err() {
            let mut st = shared.state.lock();
            st.error = Some((io::ErrorKind::Other, "cannot start pipe reader".to_string()));
            st.eof = true;
        }
        PipeInputStream { shared }
    }
}

impl InputStream for PipeInputStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut st = self.shared.state.lock();
        loop {
            if !st.buf.is_empty() {
                let n = buf.len().min(st.buf.len());
                for (i, b) in st.buf.drain(..n).enumerate() {
                    buf[i] = b;
                }
                return Ok(n);
            }
            if st.eof {
                if let Some((kind, message)) = &st.error {
                    return Err(io::Error::new(*kind, message.clone()));
                }
                return Ok(0);
            }
            self.shared.cv.wait(&mut st);
        }
    }

    fn available(&mut self) -> io::Result<usize> {
        Ok(self.shared.state.lock().buf.len())
    }
}

/// An in-memory stream (`ByteArrayInputStream`), for tests.
pub struct ByteArrayInputStream {
    data: Vec<u8>,
    pos: usize,
}

impl ByteArrayInputStream {
    pub fn new(data: Vec<u8>) -> ByteArrayInputStream {
        ByteArrayInputStream { data, pos: 0 }
    }
}

impl InputStream for ByteArrayInputStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len().min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }

    fn available(&mut self) -> io::Result<usize> {
        Ok(self.data.len() - self.pos)
    }
}

// Java: com/google/devtools/build/lib/worker/RecordingInputStream.java:41 RecordingInputStream.MAX_HEX_LINES
/// In hexdump output, the maximum number of lines to output.
const MAX_HEX_LINES: usize = 64;
// Java: com/google/devtools/build/lib/worker/RecordingInputStream.java:43 RecordingInputStream.BYTES_PER_HEX_LINE
/// In hexdump output, the number of bytes that fit on one line.
const BYTES_PER_HEX_LINE: usize = 16;
// Java: com/google/devtools/build/lib/worker/RecordingInputStream.java:45 RecordingInputStream.BYTES_PER_HEX_BLOCK
/// In hexdump output, the number of bytes that is grouped together in blocks.
const BYTES_PER_HEX_BLOCK: usize = 8;

/// An input stream filter that records the first X bytes read from its wrapped stream.
///
/// The number bytes to record can be set via [`RecordingInputStream::start_recording`], which
/// also discards any already recorded data. The recorded data can be retrieved via
/// [`RecordingInputStream::get_recorded_data_as_string`].
// Java: com/google/devtools/build/lib/worker/RecordingInputStream.java:36 RecordingInputStream
pub struct RecordingInputStream {
    input: Box<dyn InputStream>,
    recorded_data: Option<Vec<u8>>,
    max_recorded_size: usize,
}

impl RecordingInputStream {
    // Java: com/google/devtools/build/lib/worker/RecordingInputStream.java:50 RecordingInputStream.<init>
    pub fn new(input: Box<dyn InputStream>) -> RecordingInputStream {
        RecordingInputStream {
            input,
            recorded_data: None,
            max_recorded_size: 0,
        }
    }

    /// Returns the maximum number of bytes that can still be recorded in our buffer (but not more
    /// than `size`).
    // Java: com/google/devtools/build/lib/worker/RecordingInputStream.java:58 RecordingInputStream.getRecordableBytes
    fn get_recordable_bytes(&self, size: usize) -> usize {
        match &self.recorded_data {
            None => 0,
            Some(d) => (self.max_recorded_size.saturating_sub(d.len())).min(size),
        }
    }

    /// `read(byte[], int, int)`.
    // Java: com/google/devtools/build/lib/worker/RecordingInputStream.java:80 RecordingInputStream.read
    pub fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        let bytes_read = self.input.read(b)?;
        let recordable_bytes = self.get_recordable_bytes(bytes_read);
        if recordable_bytes > 0 {
            self.recorded_data
                .as_mut()
                .expect("recording")
                .extend_from_slice(&b[..recordable_bytes]);
        }
        Ok(bytes_read)
    }

    /// `available()` of the wrapped stream.
    pub fn available(&mut self) -> io::Result<usize> {
        self.input.available()
    }

    // Java: com/google/devtools/build/lib/worker/RecordingInputStream.java:90 RecordingInputStream.startRecording
    pub fn start_recording(&mut self, max_size: usize) {
        self.recorded_data = Some(Vec::with_capacity(max_size));
        self.max_recorded_size = max_size;
    }

    /// Reads whatever remaining data is available on the input stream if we still have space
    /// left in the recording buffer, in order to maximize the usefulness of the recorded data for
    /// the caller.
    // Java: com/google/devtools/build/lib/worker/RecordingInputStream.java:100 RecordingInputStream.readRemaining
    pub fn read_remaining(&mut self) {
        let Ok(available) = self.available() else {
            // Ignore.
            return;
        };
        let mut dummy = vec![0u8; self.get_recordable_bytes(available)];
        // Java's read(byte[0]) returns 0 without reading.
        if !dummy.is_empty() {
            // Ignore errors.
            let _ = self.read(&mut dummy);
        }
    }

    /// Returns the recorded data as a string, where non-printable characters are replaced with a
    /// '?' symbol. Or, if the data is not UTF-8, or has non-printable chars in the start, returns
    /// hex values formatted similarly to `hexdump -C`.
    // Java: com/google/devtools/build/lib/worker/RecordingInputStream.java:114 RecordingInputStream.getRecordedDataAsString
    pub fn get_recorded_data_as_string(&self) -> String {
        let empty = Vec::new();
        let bytes = self.recorded_data.as_ref().unwrap_or(&empty);
        // new String(bytes, UTF_8): a Unicode string (malformed input becomes U+FFFD).
        let input = String::from_utf8_lossy(bytes).into_owned();
        // TODO: Why do we get so much noise?
        let well_formed = std::str::from_utf8(bytes).is_ok();
        // input.substring(0, Math.min(input.length(), 16 * 64)): UTF-16 units.
        let prefix = utf16_prefix(&input, BYTES_PER_HEX_LINE * MAX_HEX_LINES);
        if well_formed && !prefix.chars().any(is_non_printable) {
            input
                .chars()
                .map(|c| if is_non_printable(c) { '?' } else { c })
                .collect()
        } else {
            let mut chunks: Vec<&[u8]> = Vec::with_capacity(MAX_HEX_LINES);
            while chunks.len() * BYTES_PER_HEX_LINE < bytes.len() && chunks.len() < MAX_HEX_LINES {
                let start = chunks.len() * BYTES_PER_HEX_LINE;
                let end = ((1 + chunks.len()) * BYTES_PER_HEX_LINE).min(bytes.len());
                chunks.push(&bytes[start..end]);
            }
            let is_truncated = bytes.len() > BYTES_PER_HEX_LINE * MAX_HEX_LINES;
            let lines: Vec<String> = chunks.iter().map(|c| format_hex_line(c)).collect();
            format!(
                "Not UTF-8, printing {}as hex\n{}\n",
                if is_truncated { "first 1024 bytes " } else { "" },
                lines.join("\n")
            )
        }
    }
}

/// The first `n` UTF-16 code units of `s` (Java `substring(0, n)`), as chars.
fn utf16_prefix(s: &str, n: usize) -> String {
    let units: Vec<u16> = s.encode_utf16().take(n).collect();
    String::from_utf16_lossy(&units)
}

/// `[^\p{Print}\t\r\n]` with `UNICODE_CHARACTER_CLASS`: `\p{Print}` is `[\p{Graph}\p{Blank}&&
/// [^\p{Cntrl}]]`, where `\p{Graph}` excludes whitespace, controls, surrogates and unassigned
/// code points, and `\p{Blank}` is `[\p{Zs}\t]`.
///
/// PORT: Rust has no Unicode general categories in std; graph is approximated as "not
/// whitespace and not control", which agrees with Java for every assigned code point outside
/// the format (Cf) and private-use categories.
// Java: com/google/devtools/build/lib/worker/RecordingInputStream.java:38 RecordingInputStream.NON_PRINTABLE_CHARS
fn is_non_printable(c: char) -> bool {
    if c == '\t' || c == '\r' || c == '\n' {
        return false;
    }
    if c.is_control() {
        return true;
    }
    // \p{Blank}: space separators and tab.
    let blank = c == ' ' || (c.is_whitespace() && !matches!(c, '\u{2028}' | '\u{2029}'));
    if blank {
        return false;
    }
    if c.is_whitespace() {
        return true;
    }
    // Format characters and the replacement of malformed input are not graphic in Java either
    // way; U+FFFD (So) is graphic.
    matches!(c, '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}')
}

/// Formats a single array of 16 bytes as a hexdump-style line.
// Java: com/google/devtools/build/lib/worker/RecordingInputStream.java:144 RecordingInputStream.formatHexLine
fn format_hex_line(bytes: &[u8]) -> String {
    // BaseEncoding.base16(): upper case.
    let raw_hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
    // Adds spaces between hex representation of each char
    let separated_hex = split_fixed(&raw_hex, 2).join(" ");
    // Adds extra space between each block of 8 hex bytes (two hex chars and one space each).
    let grouped_hex = split_fixed(&separated_hex, 3 * BYTES_PER_HEX_BLOCK).join(" ");
    // Adds ASCII-safe display of text on the right
    let text_display: String = bytes
        .iter()
        .map(|&b| {
            // `b >= 32` on a signed byte: 32..=127 print as the char (char)(byte) b.
            if (b as i8) >= 32 {
                (b as char).to_string()
            } else {
                ".".to_string()
            }
        })
        .collect();
    // Adds space in text display between blocks of 8 hex bytes.
    let split_text = split_fixed(&text_display, BYTES_PER_HEX_BLOCK).join(" ");
    format!("{grouped_hex:<50}|{split_text:<17}|")
}

/// `Splitter.fixedLength(n).split(s)` (chars).
fn split_fixed(s: &str, n: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    chars.chunks(n).map(|c| c.iter().collect()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Java tests: src/test/java/com/google/devtools/build/lib/worker/RecordingInputStreamTest.java

    fn stream(data: Vec<u8>) -> RecordingInputStream {
        RecordingInputStream::new(Box::new(ByteArrayInputStream::new(data)))
    }

    #[test]
    fn get_recorded_data_as_string_returns_plain_strings_as_strings() {
        let s = "A good string\nWith two lines\n";
        let mut r#in = stream(s.as_bytes().to_vec());
        r#in.start_recording(1000);
        r#in.read_remaining();
        assert_eq!(r#in.get_recorded_data_as_string(), s);
    }

    #[test]
    fn get_recorded_data_as_string_returns_non_utf8_as_hex() {
        let mut r#in = stream(vec![0xFF, 0xFE, 0x01]);
        let mut in_buf = [0u8; 1000];
        r#in.start_recording(1000);
        r#in.read(&mut in_buf).unwrap();
        assert_eq!(
            r#in.get_recorded_data_as_string(),
            "Not UTF-8, printing as hex\n\
             FF FE 01                                          |...              |\n"
        );
    }

    #[test]
    fn get_recorded_data_as_string_returns_mixed_as_hex() {
        let mut s = "One 17-char line!".to_string();
        // Doubles the length on each iteration
        for _ in 0..6 {
            s = format!("{s}{s}");
        }
        let mut bytes = s.into_bytes();
        bytes[0] = 0x00;
        bytes[1] = 0x01;
        let mut r#in = stream(bytes);
        let mut in_buf = [0u8; 1025];
        r#in.start_recording(1025);
        r#in.read(&mut in_buf).unwrap();
        assert!(r#in.get_recorded_data_as_string().starts_with(
            "Not UTF-8, printing first 1024 bytes as hex\n\
             00 01 65 20 31 37 2D 63  68 61 72 20 6C 69 6E 65  |..e 17-c har line|"
        ));
    }

    #[test]
    fn get_recorded_data_as_string_returns_long_non_utf8_as_hex_lines() {
        let mut buf: Vec<u8> = (0..25).collect();
        buf[0] = 0xFF;
        buf[1] = 0xFE;
        let mut r#in = stream(buf);
        let mut in_buf = [0u8; 1000];
        r#in.start_recording(1000);
        r#in.read(&mut in_buf).unwrap();
        assert!(r#in.get_recorded_data_as_string().starts_with(
            "Not UTF-8, printing as hex\n\
             FF FE 02 03 04 05 06 07  08 09 0A 0B 0C 0D 0E 0F  |........ ........|\n\
             10 11 12 13 14 15 16 17  18                       |........ .       |\n"
        ));
    }
}
