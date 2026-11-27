//! Netty's decoder of a `Content-Encoding: snappy` response: `SnappyFrameDecoder` (the framing
//! format, checksums not validated, as `HttpContentDecompressor` creates it with `new
//! SnappyFrameDecoder()`) over `Snappy.decode` (the raw format).
//!
//! PORT: Netty 4.1.119 (the oracle's), read from its class files (no source at the pin). The
//! decoder is a `Read` adapter over the response body instead of a channel handler: it reads one
//! chunk at a time and hands out its data. A `DecompressionException` (a `RuntimeException` in
//! Netty) is an `io::Error` carrying that class name ([`JavaException`]).

use std::io::{self, Read};

use crate::common::errors::JavaException;

// Java: io/netty/handler/codec/compression/SnappyFrameDecoder.java SnappyFrameDecoder.SNAPPY_IDENTIFIER_LEN
const SNAPPY_IDENTIFIER_LEN: usize = 6;
// Java: io/netty/handler/codec/compression/SnappyFrameDecoder.java SnappyFrameDecoder.MAX_UNCOMPRESSED_DATA_SIZE
const MAX_UNCOMPRESSED_DATA_SIZE: usize = 65536 + 4;
// Java: io/netty/handler/codec/compression/SnappyFrameDecoder.java SnappyFrameDecoder.MAX_DECOMPRESSED_DATA_SIZE
const MAX_DECOMPRESSED_DATA_SIZE: usize = 65536;
// Java: io/netty/handler/codec/compression/SnappyFrameDecoder.java SnappyFrameDecoder.MAX_COMPRESSED_CHUNK_SIZE
const MAX_COMPRESSED_CHUNK_SIZE: usize = 16777216 - 1;

fn decompression_exception(message: impl Into<String>) -> io::Error {
    io::Error::other(JavaException {
        class_name: "io.netty.handler.codec.compression.DecompressionException",
        message: message.into(),
    })
}

/// `SnappyFrameDecoder.ChunkType`.
enum ChunkType {
    StreamIdentifier,
    CompressedData,
    UncompressedData,
    ReservedUnskippable,
    ReservedSkippable,
}

// Java: io/netty/handler/codec/compression/SnappyFrameDecoder.java SnappyFrameDecoder.mapChunkType
fn map_chunk_type(chunk_type: u8) -> ChunkType {
    match chunk_type {
        0 => ChunkType::CompressedData,
        1 => ChunkType::UncompressedData,
        0xff => ChunkType::StreamIdentifier,
        t if t & 0x80 == 0x80 => ChunkType::ReservedSkippable,
        _ => ChunkType::ReservedUnskippable,
    }
}

/// `SnappyFrameDecoder` as a reader of the decoded data.
// Java: io/netty/handler/codec/compression/SnappyFrameDecoder.java SnappyFrameDecoder
pub(crate) struct SnappyFrameDecoder<R> {
    input: R,
    started: bool,
    /// Decoded data not yet read.
    pending: Vec<u8>,
    pending_offset: usize,
}

impl<R: Read> SnappyFrameDecoder<R> {
    pub(crate) fn new(input: R) -> SnappyFrameDecoder<R> {
        SnappyFrameDecoder {
            input,
            started: false,
            pending: Vec::new(),
            pending_offset: 0,
        }
    }

    /// Reads exactly `buf.len()` bytes; `Ok(false)` at the end of the input before any of them
    /// (a chunk that never completes is left undecoded, as Netty keeps it in its cumulation).
    fn read_exactly(&mut self, buf: &mut [u8]) -> io::Result<bool> {
        let mut filled = 0;
        while filled < buf.len() {
            match self.input.read(&mut buf[filled..]) {
                Ok(0) => return Ok(false),
                Ok(n) => filled += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }

    /// `decode(ctx, in, out)` for one chunk: `Ok(false)` at the end of the input.
    // Java: io/netty/handler/codec/compression/SnappyFrameDecoder.java SnappyFrameDecoder.decode
    fn decode_chunk(&mut self) -> io::Result<bool> {
        let mut header = [0u8; 4];
        if !self.read_exactly(&mut header)? {
            return Ok(false);
        }
        let chunk_type_val = header[0];
        let chunk_length =
            usize::from(header[1]) | usize::from(header[2]) << 8 | usize::from(header[3]) << 16;
        match map_chunk_type(chunk_type_val) {
            ChunkType::StreamIdentifier => {
                if chunk_length != SNAPPY_IDENTIFIER_LEN {
                    return Err(decompression_exception(format!(
                        "Unexpected length of stream identifier: {chunk_length}"
                    )));
                }
                let mut identifier = [0u8; SNAPPY_IDENTIFIER_LEN];
                if !self.read_exactly(&mut identifier)? {
                    return Ok(false);
                }
                if &identifier != b"sNaPpY" {
                    return Err(decompression_exception(
                        "Unexpected stream identifier contents. Mismatched snappy protocol version?",
                    ));
                }
                self.started = true;
            }
            ChunkType::ReservedSkippable => {
                if !self.started {
                    return Err(decompression_exception(
                        "Received RESERVED_SKIPPABLE tag before STREAM_IDENTIFIER",
                    ));
                }
                io::copy(
                    &mut (&mut self.input).take(chunk_length as u64),
                    &mut io::sink(),
                )?;
            }
            ChunkType::ReservedUnskippable => {
                return Err(decompression_exception(format!(
                    "Found reserved unskippable chunk type: 0x{chunk_type_val:x}"
                )));
            }
            ChunkType::UncompressedData => {
                if !self.started {
                    return Err(decompression_exception(
                        "Received UNCOMPRESSED_DATA tag before STREAM_IDENTIFIER",
                    ));
                }
                if chunk_length > MAX_UNCOMPRESSED_DATA_SIZE {
                    return Err(decompression_exception(format!(
                        "Received UNCOMPRESSED_DATA larger than {MAX_UNCOMPRESSED_DATA_SIZE} bytes"
                    )));
                }
                let mut chunk = vec![0u8; chunk_length];
                if !self.read_exactly(&mut chunk)? {
                    return Ok(false);
                }
                // The checksum (4 bytes) is skipped: validateChecksums is false.
                self.pending = chunk.split_off(4.min(chunk.len()));
                self.pending_offset = 0;
            }
            ChunkType::CompressedData => {
                if !self.started {
                    return Err(decompression_exception(
                        "Received COMPRESSED_DATA tag before STREAM_IDENTIFIER",
                    ));
                }
                if chunk_length > MAX_COMPRESSED_CHUNK_SIZE {
                    return Err(decompression_exception(format!(
                        "Received COMPRESSED_DATA that contains chunk that exceeds {MAX_COMPRESSED_CHUNK_SIZE} bytes"
                    )));
                }
                let mut chunk = vec![0u8; chunk_length];
                if !self.read_exactly(&mut chunk)? {
                    return Ok(false);
                }
                let data = chunk.get(4..).unwrap_or(&[]);
                // snappy.getPreamble(in): the uncompressed length, without consuming it.
                let uncompressed_size = read_preamble(&mut &data[..])?;
                if uncompressed_size > MAX_DECOMPRESSED_DATA_SIZE {
                    return Err(decompression_exception(format!(
                        "Received COMPRESSED_DATA that contains uncompressed data that exceeds {MAX_DECOMPRESSED_DATA_SIZE} bytes"
                    )));
                }
                self.pending = snappy_decode(data)?;
                self.pending_offset = 0;
            }
        }
        Ok(true)
    }
}

impl<R: Read> Read for SnappyFrameDecoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.pending_offset == self.pending.len() {
            self.pending.clear();
            self.pending_offset = 0;
            if !self.decode_chunk()? {
                return Ok(0);
            }
        }
        let n = buf.len().min(self.pending.len() - self.pending_offset);
        buf[..n].copy_from_slice(&self.pending[self.pending_offset..self.pending_offset + n]);
        self.pending_offset += n;
        Ok(n)
    }
}

/// `Snappy.readPreamble`: the varint uncompressed length; 0 for an empty input.
// Java: io/netty/handler/codec/compression/Snappy.java Snappy.readPreamble
fn read_preamble(input: &mut &[u8]) -> io::Result<usize> {
    let mut length: usize = 0;
    let mut byte_index = 0;
    while let Some((&current, rest)) = input.split_first() {
        *input = rest;
        length |= usize::from(current & 0x7f) << (byte_index * 7);
        byte_index += 1;
        if current & 0x80 == 0 {
            return Ok(length);
        }
        if byte_index >= 4 {
            return Err(decompression_exception("Preamble is greater than 4 bytes"));
        }
    }
    Ok(0)
}

/// `Snappy.validateOffset(offset, chunkSizeSoFar)`.
// Java: io/netty/handler/codec/compression/Snappy.java Snappy.validateOffset
fn validate_offset(offset: i32, chunk_size_so_far: usize) -> io::Result<()> {
    if offset == 0 {
        return Err(decompression_exception(
            "Offset is less than minimum permissible value",
        ));
    }
    if offset < 0 {
        return Err(decompression_exception(
            "Offset is greater than maximum value supported by this implementation",
        ));
    }
    if offset as usize > chunk_size_so_far {
        return Err(decompression_exception("Offset exceeds size of chunk"));
    }
    Ok(())
}

/// `Snappy.decode(in, out)` of one compressed chunk: the preamble, then literals and copies. A
/// preamble of 0 ends the decoding; an element cut off by the end of the input ends it too (Netty
/// waits for more input, which never comes within a chunk).
///
/// PORT: Netty's output buffer has a maximum capacity of 65536 (`alloc.buffer(size, 65536)`);
/// writing past it throws an `IndexOutOfBoundsException`, which here is a
/// `DecompressionException`-classed error with Netty's index message replaced by a fixed text.
// Java: io/netty/handler/codec/compression/Snappy.java Snappy.decode
fn snappy_decode(mut input: &[u8]) -> io::Result<Vec<u8>> {
    let preamble = read_preamble(&mut input)?;
    let mut out: Vec<u8> = Vec::with_capacity(preamble);
    if preamble == 0 {
        return Ok(out);
    }
    let too_large = || {
        io::Error::other(JavaException {
            class_name: "java.lang.IndexOutOfBoundsException",
            message: "the decoded chunk exceeds 65536 bytes".to_string(),
        })
    };
    while let Some((&tag, rest)) = input.split_first() {
        input = rest;
        match tag & 0x03 {
            // Snappy.decodeLiteral
            0 => {
                let length_bytes = match (tag >> 2) & 0x3f {
                    60 => 1,
                    61 => 2,
                    62 => 3,
                    63 => 4,
                    _ => 0,
                };
                let mut length: usize = if length_bytes == 0 {
                    usize::from((tag >> 2) & 0x3f)
                } else {
                    if input.len() < length_bytes {
                        break;
                    }
                    let mut value: u32 = 0;
                    for (i, b) in input[..length_bytes].iter().enumerate() {
                        value |= u32::from(*b) << (8 * i);
                    }
                    input = &input[length_bytes..];
                    // readIntLE() is a signed int; a negative length is never readable.
                    if length_bytes == 4 && (value as i32) < 0 {
                        break;
                    }
                    value as usize
                };
                length += 1;
                if input.len() < length {
                    break;
                }
                if out.len() + length > MAX_DECOMPRESSED_DATA_SIZE {
                    return Err(too_large());
                }
                out.extend_from_slice(&input[..length]);
                input = &input[length..];
            }
            kind => {
                let (length, offset): (usize, i32) = match kind {
                    // Snappy.decodeCopyWith1ByteOffset
                    1 => {
                        let Some((&b, rest)) = input.split_first() else {
                            break;
                        };
                        input = rest;
                        (
                            4 + usize::from((tag & 0x1c) >> 2),
                            ((i32::from(tag) & 0xe0) << 8 >> 5) | i32::from(b),
                        )
                    }
                    // Snappy.decodeCopyWith2ByteOffset
                    2 => {
                        if input.len() < 2 {
                            break;
                        }
                        let offset = i32::from(u16::from_le_bytes([input[0], input[1]]));
                        input = &input[2..];
                        (1 + usize::from((tag >> 2) & 0x3f), offset)
                    }
                    // Snappy.decodeCopyWith4ByteOffset
                    _ => {
                        if input.len() < 4 {
                            break;
                        }
                        let offset = i32::from_le_bytes([input[0], input[1], input[2], input[3]]);
                        input = &input[4..];
                        (1 + usize::from((tag >> 2) & 0x3f), offset)
                    }
                };
                validate_offset(offset, out.len())?;
                if out.len() + length > MAX_DECOMPRESSED_DATA_SIZE {
                    return Err(too_large());
                }
                let start = out.len() - offset as usize;
                // Overlapping copies repeat the last `offset` bytes, as Netty's repeated reads.
                for i in 0..length {
                    let byte = out[start + i];
                    out.push(byte);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(chunks: &[(u8, &[u8])]) -> Vec<u8> {
        let mut out = vec![0xff, 6, 0, 0];
        out.extend_from_slice(b"sNaPpY");
        for (kind, body) in chunks {
            let len = body.len();
            out.extend_from_slice(&[*kind, len as u8, (len >> 8) as u8, (len >> 16) as u8]);
            out.extend_from_slice(body);
        }
        out
    }

    fn decode_all(bytes: &[u8]) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        SnappyFrameDecoder::new(bytes).read_to_end(&mut out)?;
        Ok(out)
    }

    #[test]
    fn uncompressed_and_compressed_chunks() {
        // Uncompressed: 4 checksum bytes (not validated), then "hello ".
        let mut uncompressed = vec![0, 0, 0, 0];
        uncompressed.extend_from_slice(b"hello ");
        // Compressed: checksum, preamble 9, literal "abc" (tag 2 << 2), copy1 of 6 bytes at
        // offset 3 (tag 0b000_010_01, offset byte 3): "abcabcabc".
        let compressed: &[u8] = &[0, 0, 0, 0, 9, 2 << 2, b'a', b'b', b'c', 0b0000_1001, 3];
        let bytes = frame(&[(1, &uncompressed), (0xfe, b"pad"), (0, compressed)]);
        assert_eq!(decode_all(&bytes).unwrap(), b"hello abcabcabc");
    }

    #[test]
    fn data_before_the_stream_identifier_is_refused() {
        let e = decode_all(&[1, 4, 0, 0, 0, 0, 0, 0]).unwrap_err();
        assert_eq!(
            e.to_string(),
            "Received UNCOMPRESSED_DATA tag before STREAM_IDENTIFIER"
        );
    }

    #[test]
    fn reserved_unskippable_chunk_is_refused() {
        let e = decode_all(&frame(&[(0x02, b"")])).unwrap_err();
        assert_eq!(e.to_string(), "Found reserved unskippable chunk type: 0x2");
    }

    #[test]
    fn copy_offset_beyond_the_output_is_refused() {
        let compressed: &[u8] = &[0, 0, 0, 0, 5, 0b0000_0101, 9];
        let e = decode_all(&frame(&[(0, compressed)])).unwrap_err();
        assert_eq!(e.to_string(), "Offset exceeds size of chunk");
    }
}
