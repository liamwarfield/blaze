//! Port of `lib/util/io/MessageInputStreamWrapper.java`.

use std::io::{self, BufReader, Read};
use std::marker::PhantomData;

use blaze_protos::JavaMessage;
use blaze_protos::json::{JsonFormat, Parser};

use super::MessageInputStream;

/// `InvalidProtocolBufferException.truncatedMessage()`.
// Java: com/google/protobuf/InvalidProtocolBufferException.java truncatedMessage
fn truncated_message() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "While parsing a protocol message, the input ended unexpectedly in the middle of a field.  \
         This could mean either that the input has been truncated or that an embedded message \
         misreported its own length.",
    )
}

/// `InvalidProtocolBufferException.malformedVarint()`.
fn malformed_varint() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "CodedInputStream encountered a malformed varint.",
    )
}

/// Reads one byte; `Ok(None)` at end of input.
fn read_byte(input: &mut dyn Read) -> io::Result<Option<u8>> {
    let mut b = [0u8; 1];
    loop {
        match input.read(&mut b) {
            Ok(0) => return Ok(None),
            Ok(_) => return Ok(Some(b[0])),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
}

/// `MessageLite.parseDelimitedFrom(InputStream)`: reads a varint length and then a message of that
/// many bytes. Returns `Ok(None)` if the stream is at end of input before the first byte.
// Java: com/google/protobuf/AbstractParser.java parsePartialDelimitedFrom
pub fn parse_delimited_from<M: JavaMessage>(input: &mut dyn Read) -> io::Result<Option<M>> {
    let Some(first_byte) = read_byte(input)? else {
        return Ok(None);
    };
    // `CodedInputStream.readRawVarint32(firstByte, input)`.
    let mut size: u32 = (first_byte & 0x7f) as u32;
    if first_byte & 0x80 != 0 {
        let mut offset = 7;
        let mut done = false;
        while offset < 32 {
            let b = read_byte(input)?.ok_or_else(truncated_message)?;
            size |= ((b & 0x7f) as u32) << offset;
            if b & 0x80 == 0 {
                done = true;
                break;
            }
            offset += 7;
        }
        if !done {
            // Keep reading up to 64 bits.
            while offset < 64 {
                let b = read_byte(input)?.ok_or_else(truncated_message)?;
                if b & 0x80 == 0 {
                    done = true;
                    break;
                }
                offset += 7;
            }
            if !done {
                return Err(malformed_varint());
            }
        }
    }
    let size = size as i32;
    if size < 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "CodedInputStream encountered an embedded string or message which claimed to have \
             negative size.",
        ));
    }
    let mut buf = vec![0u8; size as usize];
    let mut read = 0;
    while read < buf.len() {
        match input.read(&mut buf[read..]) {
            Ok(0) => return Err(truncated_message()),
            Ok(n) => read += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    // PORT: decode errors carry prost's message, not protobuf-java's.
    <M as prost::Message>::decode(&buf[..])
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}

/// Reads the messages in length-delimited protobuf wire format.
// Java: com/google/devtools/build/lib/util/io/MessageInputStreamWrapper.java:39 MessageInputStreamWrapper.BinaryInputStreamWrapper
pub struct BinaryInputStreamWrapper<T> {
    // PORT: buffered, since the varint prefix is read byte by byte.
    stream: Option<BufReader<Box<dyn Read + Send>>>,
    _marker: PhantomData<fn() -> T>,
}

impl<T> BinaryInputStreamWrapper<T> {
    // Java: com/google/devtools/build/lib/util/io/MessageInputStreamWrapper.java:44 BinaryInputStreamWrapper.<init>
    pub fn new(stream: Box<dyn Read + Send>) -> BinaryInputStreamWrapper<T> {
        BinaryInputStreamWrapper {
            stream: Some(BufReader::new(stream)),
            _marker: PhantomData,
        }
    }
}

impl<T: JavaMessage> MessageInputStream<T> for BinaryInputStreamWrapper<T> {
    // Java: com/google/devtools/build/lib/util/io/MessageInputStreamWrapper.java:50 BinaryInputStreamWrapper.read
    fn read(&mut self) -> io::Result<Option<T>> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| io::Error::other("Stream Closed"))?;
        parse_delimited_from(stream)
    }

    // Java: com/google/devtools/build/lib/util/io/MessageInputStreamWrapper.java:55 BinaryInputStreamWrapper.close
    fn close(&mut self) -> io::Result<()> {
        self.stream = None;
        Ok(())
    }
}

/// Reads the messages in concatenated JSON text format.
// Java: com/google/devtools/build/lib/util/io/MessageInputStreamWrapper.java:61 MessageInputStreamWrapper.JsonInputStreamWrapper
pub struct JsonInputStreamWrapper<T> {
    stream: Option<Box<dyn Read + Send>>,
    // The tokens not yet returned (the whole input is tokenized on the first read).
    tokens: Option<std::collections::VecDeque<String>>,
    parser: Parser,
    _marker: PhantomData<fn() -> T>,
}

impl<T> JsonInputStreamWrapper<T> {
    // Java: com/google/devtools/build/lib/util/io/MessageInputStreamWrapper.java:71 JsonInputStreamWrapper.<init>
    pub fn new(stream: Box<dyn Read + Send>) -> JsonInputStreamWrapper<T> {
        JsonInputStreamWrapper {
            stream: Some(stream),
            tokens: None,
            // Java: com/google/devtools/build/lib/util/io/MessageInputStreamWrapper.java:62 JsonInputStreamWrapper.PARSER
            parser: JsonFormat::parser().ignoring_unknown_fields(),
            _marker: PhantomData,
        }
    }

    /// `new Scanner(stream, UTF_8).useDelimiter("(?<=\\n\\})(?=\\{\\n)")`: the string `\n}{\n`
    /// is a reliable delimiter; the braces stay with the tokens.
    // Java: com/google/devtools/build/lib/util/io/MessageInputStreamWrapper.java:66 JsonInputStreamWrapper.DELIMITER
    fn tokenize(text: &str) -> std::collections::VecDeque<String> {
        let mut tokens = std::collections::VecDeque::new();
        let mut start = 0;
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if i >= 2
                && bytes[i - 2] == b'\n'
                && bytes[i - 1] == b'}'
                && bytes[i..].starts_with(b"{\n")
            {
                if i > start {
                    tokens.push_back(text[start..i].to_string());
                }
                start = i;
            }
            i += 1;
        }
        if start < bytes.len() {
            tokens.push_back(text[start..].to_string());
        }
        tokens
    }
}

impl<T: JavaMessage> MessageInputStream<T> for JsonInputStreamWrapper<T> {
    // Java: com/google/devtools/build/lib/util/io/MessageInputStreamWrapper.java:77 JsonInputStreamWrapper.read
    fn read(&mut self) -> io::Result<Option<T>> {
        if self.tokens.is_none() {
            let stream = self
                .stream
                .as_mut()
                .ok_or_else(|| io::Error::other("Scanner closed"))?;
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes)?;
            // `new Scanner(stream, UTF_8)`: the JDK's UTF-8 decoder replaces malformed input.
            let text = blaze_util::string_encoding::java_new_string_utf8(&bytes);
            self.tokens = Some(Self::tokenize(&text));
        }
        let Some(token) = self.tokens.as_mut().and_then(|t| t.pop_front()) else {
            return Ok(None);
        };
        let mut builder = T::default();
        self.parser
            .merge(&token, &mut builder)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.get_message().to_string()))?;
        Ok(Some(builder))
    }

    // Java: com/google/devtools/build/lib/util/io/MessageInputStreamWrapper.java:87 JsonInputStreamWrapper.close
    fn close(&mut self) -> io::Result<()> {
        self.stream = None;
        Ok(())
    }
}
