#![cfg(feature = "wip")]
// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/zstd/ZstdCompressingInputStreamTest.java`
//! and `ZstdDecompressingOutputStreamTest.java`.
//!
//! zstd-jni's `Zstd.compress(data)` / `Zstd.decompress(bytes, originalSize)` are
//! `zstd::bulk::compress(data, DEFAULT_COMPRESSION_LEVEL)` / `zstd::bulk::decompress`.

use std::io::{self, Read, Write};

use blaze_remote::zstd::{ZstdCompressingInputStream, ZstdDecompressingOutputStream};

/// `new Random().nextBytes(data)`.
fn random_bytes(len: usize) -> Vec<u8> {
    use std::hash::{BuildHasher, Hasher};
    let mut state = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
        | 1;
    (0..len)
        .map(|_| {
            // xorshift64
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

/// `ByteStreams.toByteArray(in)`.
fn to_byte_array(mut in_: impl Read) -> Vec<u8> {
    let mut out = Vec::new();
    in_.read_to_end(&mut out).unwrap();
    out
}

/// `Zstd.compress(data)`.
fn zstd_compress(data: &[u8]) -> Vec<u8> {
    zstd::bulk::compress(data, zstd::DEFAULT_COMPRESSION_LEVEL).unwrap()
}

// ZstdCompressingInputStreamTest

#[test]
fn compression_works() {
    let data = random_bytes(50);

    let bais = io::Cursor::new(data.clone());
    let zdis = ZstdCompressingInputStream::new(bais).unwrap();
    assert_eq!(
        zstd::bulk::decompress(&to_byte_array(zdis), data.len()).unwrap(),
        data
    );
}

#[test]
fn stream_can_be_compressed_with_minimum_buffer_size() {
    let data = random_bytes(50);

    let bais = io::Cursor::new(data.clone());
    let zdis = ZstdCompressingInputStream::with_size(
        bais,
        ZstdCompressingInputStream::<io::Cursor<Vec<u8>>>::MIN_BUFFER_SIZE,
    )
    .unwrap();
    assert_eq!(
        zstd::bulk::decompress(&to_byte_array(zdis), data.len()).unwrap(),
        data
    );
}

// ZstdDecompressingOutputStreamTest

#[test]
fn decompression_works() {
    let data = random_bytes(50);
    let compressed = zstd_compress(&data);

    let mut zdos = ZstdDecompressingOutputStream::new(Vec::<u8>::new()).unwrap();
    zdos.write_bytes(&compressed).unwrap();
    zdos.flush().unwrap();
    let baos = zdos.close().unwrap();

    assert_eq!(baos, data);
}

#[test]
fn stream_can_be_decompressed_one_byte_at_a_time() {
    let data = random_bytes(50);
    let compressed = zstd_compress(&data);

    let mut zdos = ZstdDecompressingOutputStream::new(Vec::<u8>::new()).unwrap();
    for b in &compressed {
        zdos.write_byte(*b).unwrap();
    }
    zdos.flush().unwrap();
    let baos = zdos.close().unwrap();

    assert_eq!(baos, data);
}

#[test]
fn bytes_written_matches_decompressed_bytes() {
    let data = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".as_bytes();

    // A `ZstdOutputStream` with `setCloseFrameOnFlush(true)`: every flush ends the frame that
    // holds the bytes written since the previous flush, and `close()` ends the last one.
    let mut compressed: Vec<u8> = Vec::new();
    let mut frame: Vec<u8> = Vec::new();
    fn close_frame(frame: &mut Vec<u8>, compressed: &mut Vec<u8>) {
        if !frame.is_empty() {
            let mut zos =
                zstd::stream::write::Encoder::new(Vec::new(), zstd::DEFAULT_COMPRESSION_LEVEL)
                    .unwrap();
            zos.write_all(frame).unwrap();
            compressed.extend_from_slice(&zos.finish().unwrap());
            frame.clear();
        }
    }
    for (i, b) in data.iter().enumerate() {
        frame.push(*b);
        if i % 5 == 0 {
            // Create multiple frames of 5 bytes each.
            close_frame(&mut frame, &mut compressed);
        }
    }
    close_frame(&mut frame, &mut compressed);

    let mut zdos = ZstdDecompressingOutputStream::new(Vec::<u8>::new()).unwrap();
    for b in &compressed {
        zdos.write_byte(*b).unwrap();
        zdos.flush().unwrap();
    }
    let decompressed = zdos.close().unwrap();
    assert_eq!(decompressed, data);
}
