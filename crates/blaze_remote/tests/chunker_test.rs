#![cfg(feature = "wip")]
// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/ChunkerTest.java`: tests for
//! `Chunker`.
//!
//! `assertThrows(NoSuchElementException.class, ...)` is a caught panic (an unchecked exception);
//! zstd-jni's `Zstd.decompress(bytes, originalSize)` is `zstd::bulk::decompress`.

use std::io::{self, Read};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use blaze_remote::chunker::{Chunk, Chunker};
use blaze_remote::common::remote_cache_client::Blob;
use parking_lot::Mutex;

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

/// `assertThrows(NoSuchElementException.class, () -> chunker.next())`.
fn assert_next_throws_no_such_element_exception(chunker: &mut Chunker<'_>) {
    let result = catch_unwind(AssertUnwindSafe(|| chunker.next()));
    let panic = match result {
        Ok(_) => panic!("expected a NoSuchElementException"),
        Err(panic) => panic,
    };
    let message = panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    assert!(
        message.contains("NoSuchElementException"),
        "unexpected panic: {message}"
    );
}

#[test]
fn chunking_should_work() {
    let expected_data = random_bytes(21);

    let mut chunker = Chunker::builder()
        .set_input_bytes(expected_data.clone())
        .set_chunk_size(10)
        .build();

    let mut actual_data: Vec<u8> = Vec::new();

    assert!(chunker.has_next());
    let next = chunker.next().unwrap();
    assert_eq!(next.get_offset(), 0);
    assert_eq!(next.get_data().len(), 10);
    actual_data.extend_from_slice(next.get_data());

    assert!(chunker.has_next());
    let next = chunker.next().unwrap();
    assert_eq!(next.get_offset(), 10);
    assert_eq!(next.get_data().len(), 10);
    actual_data.extend_from_slice(next.get_data());

    assert!(chunker.has_next());
    let next = chunker.next().unwrap();
    assert_eq!(next.get_offset(), 20);
    assert_eq!(next.get_data().len(), 1);
    actual_data.extend_from_slice(next.get_data());

    assert!(!chunker.has_next());

    assert_eq!(actual_data, expected_data);
}

#[test]
fn next_should_throw_if_no_more_data() {
    let data = vec![0u8; 10];
    let mut chunker = Chunker::builder()
        .set_input_bytes(data)
        .set_chunk_size(10)
        .build();

    assert!(chunker.has_next());
    let _: Chunk = chunker.next().unwrap();

    assert!(!chunker.has_next());

    assert_next_throws_no_such_element_exception(&mut chunker);
}

/// A `ByteArrayInputStream` that records being closed (dropped).
struct CloseTrackingInputStream {
    inner: io::Cursor<Vec<u8>>,
    closed: Arc<AtomicBool>,
    close_calls: Arc<AtomicI32>,
}

impl Read for CloseTrackingInputStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Drop for CloseTrackingInputStream {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        self.close_calls.fetch_add(1, Ordering::SeqCst);
    }
}

/// A `Blob` whose streams record being closed.
struct TrackingBlob {
    data: Vec<u8>,
    /// The `closed` flag and the `close()` call count of the stream opened last (`in.get()`).
    last: Mutex<Option<(Arc<AtomicBool>, Arc<AtomicI32>)>>,
}

impl TrackingBlob {
    fn new(data: &[u8]) -> TrackingBlob {
        TrackingBlob {
            data: data.to_vec(),
            last: Mutex::new(None),
        }
    }

    /// `in.get() == null`.
    fn never_opened(&self) -> bool {
        self.last.lock().is_none()
    }

    /// Whether the stream opened last was closed.
    fn last_closed(&self) -> bool {
        let last = self.last.lock();
        let (closed, _) = last.as_ref().expect("no stream was opened");
        closed.load(Ordering::SeqCst)
    }

    /// How often the stream opened last was closed.
    fn last_close_calls(&self) -> i32 {
        let last = self.last.lock();
        let (_, close_calls) = last.as_ref().expect("no stream was opened");
        close_calls.load(Ordering::SeqCst)
    }
}

impl Blob for TrackingBlob {
    fn get(&self) -> io::Result<Box<dyn Read + Send + '_>> {
        let closed = Arc::new(AtomicBool::new(false));
        let close_calls = Arc::new(AtomicI32::new(0));
        *self.last.lock() = Some((closed.clone(), close_calls.clone()));
        Ok(Box::new(CloseTrackingInputStream {
            inner: io::Cursor::new(self.data.clone()),
            closed,
            close_calls,
        }))
    }
}

#[test]
fn empty_data() {
    let inp = TrackingBlob::new(&[]);
    let mut chunker = Chunker::builder().set_input(0, &inp).build();

    assert!(chunker.has_next());

    let next = chunker.next().unwrap();

    assert!(next.get_data().is_empty());
    assert_eq!(next.get_offset(), 0);

    assert!(!chunker.has_next());
    assert!(inp.last_closed());

    assert_next_throws_no_such_element_exception(&mut chunker);
}

#[test]
fn reset() {
    let data: Vec<u8> = vec![1, 2, 3];
    let mut chunker = Chunker::builder()
        .set_input_bytes(data)
        .set_chunk_size(1)
        .build();

    assert_next_equals(&mut chunker, &[1]);
    assert_next_equals(&mut chunker, &[2]);

    chunker.reset().unwrap();

    assert_next_equals(&mut chunker, &[1]);
    assert_next_equals(&mut chunker, &[2]);
    assert_next_equals(&mut chunker, &[3]);

    chunker.reset().unwrap();

    assert_next_equals(&mut chunker, &[1]);
}

#[test]
fn resources_should_be_released() {
    // Test that after having consumed all data or after reset() is called (whatever happens first)
    // the underlying InputStream should be closed.

    let data: Vec<u8> = vec![1, 2];
    let supplier = TrackingBlob::new(&data);

    let mut chunker = Chunker::new(&supplier, data.len() as i64, 1, false);
    assert!(supplier.never_opened());
    assert_next_equals(&mut chunker, &[1]);
    assert_eq!(supplier.last_close_calls(), 0);
    assert_next_equals(&mut chunker, &[2]);
    assert_eq!(supplier.last_close_calls(), 1);

    chunker.reset().unwrap();
    chunker.next().unwrap();
    chunker.reset().unwrap();
    assert_eq!(supplier.last_close_calls(), 1);
}

#[test]
fn seek_after_reset() {
    // Test that seek() works on an uninitialized chunker

    let data = vec![0u8; 10];
    let mut chunker = Chunker::builder()
        .set_input_bytes(data)
        .set_chunk_size(10)
        .build();

    chunker.reset().unwrap();
    chunker.seek(2).unwrap();

    let next = chunker.next().unwrap();
    assert_eq!(next.get_offset(), 2);
    assert_eq!(next.get_data().len(), 8);
}

#[test]
fn seek_backwards() {
    let data = vec![0u8; 10];
    let mut chunker = Chunker::builder()
        .set_input_bytes(data)
        .set_chunk_size(10)
        .build();

    chunker.seek(4).unwrap();
    chunker.seek(2).unwrap();

    let next = chunker.next().unwrap();
    assert_eq!(next.get_offset(), 2);
    assert_eq!(next.get_data().len(), 8);
}

#[test]
fn seek_forwards() {
    let mut data = vec![0u8; 10];
    for (i, b) in data.iter_mut().enumerate() {
        *b = i as u8;
    }
    let mut chunker = Chunker::builder()
        .set_input_bytes(data)
        .set_chunk_size(2)
        .build();

    let chunk = chunker.next().unwrap();
    assert_eq!(chunk.get_offset(), 0);
    assert_eq!(&chunk.get_data()[..], &[0u8, 1][..]);
    chunker.seek(8).unwrap();
    let chunk = chunker.next().unwrap();
    assert_eq!(chunk.get_offset(), 8);
    assert_eq!(&chunk.get_data()[..], &[8u8, 9][..]);
    assert!(!chunker.has_next());
}

#[test]
fn seek_empty_data() {
    let mut chunker = Chunker::builder().set_input_bytes(Vec::<u8>::new()).build();
    for _ in 0..2 {
        chunker.seek(0).unwrap();
        let next = chunker.next().unwrap();
        assert!(next.get_data().is_empty());
        assert_eq!(next.get_offset(), 0);

        assert!(!chunker.has_next());
        assert_next_throws_no_such_element_exception(&mut chunker);
    }
}

#[test]
fn test_single_chunk_compressed() {
    let data: Vec<u8> = vec![72, 101, 108, 108, 111, 32, 87, 111, 114, 108, 100, 33];
    let mut chunker = Chunker::builder()
        .set_input_bytes(data.clone())
        .set_chunk_size(data.len() as i32 * 2)
        .set_compressed(true)
        .build();
    let next = chunker.next().unwrap();
    assert!(!chunker.has_next());
    assert_eq!(
        zstd::bulk::decompress(next.get_data(), data.len()).unwrap(),
        data
    );
}

#[test]
fn test_multi_chunk_compressed() {
    let data: Vec<u8> = vec![72, 101, 108, 108, 111, 32, 87, 111, 114, 108, 100, 33];
    let mut chunker = Chunker::builder()
        .set_input_bytes(data.clone())
        .set_chunk_size(data.len() as i32 / 2)
        .set_compressed(true)
        .build();

    let mut baos: Vec<u8> = Vec::new();
    baos.extend_from_slice(chunker.next().unwrap().get_data());
    assert!(chunker.has_next());
    while chunker.has_next() {
        baos.extend_from_slice(chunker.next().unwrap().get_data());
    }

    assert_eq!(zstd::bulk::decompress(&baos, data.len()).unwrap(), data);
}

#[test]
fn test_actual_size_is_correct_after_seek() {
    let data: Vec<u8> = vec![72, 101, 108, 108, 111, 32, 87, 111, 114, 108, 100, 33];
    let expected_sizes = [12, 24];
    for expected in expected_sizes {
        let mut chunker = Chunker::builder()
            .set_input_bytes(data.clone())
            .set_chunk_size(data.len() as i32 * 2)
            .set_compressed(expected != data.len() as i64)
            .build();
        chunker.seek(5).unwrap();
        chunker.next().unwrap();
        assert!(!chunker.has_next());
        assert_eq!(chunker.get_offset(), expected);
    }
}

fn assert_next_equals(chunker: &mut Chunker<'_>, data: &[u8]) {
    assert!(chunker.has_next());
    let next = chunker.next().unwrap();
    assert_eq!(&next.get_data()[..], data);
}
