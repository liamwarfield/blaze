//! Port of `lib/remote/ChunkedBlobDownloaderTest.java`.
//!
//! PORT: Java mocks `GrpcCacheClient` and `CombinedCache` with Mockito; the downloader's seams
//! here are the traits `ChunkSplitter` and `ChunkSource`, implemented by the fakes below. A
//! `SettableFuture` is a [`Settable`]; Java's download thread is a thread running `block_on`.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use blaze_remote::chunked_blob_downloader::{ChunkSource, ChunkSplitter, ChunkedBlobDownloader};
use blaze_remote::chunking::{ChunkingConfig, FastCdcChunkingConfig};
use blaze_remote::common::errors;
use blaze_remote::common::{
    BlobNotSplittableException, CacheNotFoundException, OutputDigestMismatchException,
    RemoteActionExecutionContext,
};
use blaze_remote::reapi::{Digest, RequestMetadata, SplitBlobResponse, chunking_function};
use blaze_remote::runtime::block_on;
use blaze_remote::util::DigestUtil;
use blaze_vfs::DigestHashFunction;
use futures::future::{BoxFuture, FutureExt, Shared};
use parking_lot::Mutex;

const MAX_IN_FLIGHT_CHUNK_DOWNLOADS: usize = 16;

fn digest_util() -> DigestUtil {
    DigestUtil::new(DigestHashFunction::Sha256)
}

fn compute(data: &[u8]) -> Digest {
    digest_util().compute(data)
}

type Outcome = Result<Vec<u8>, String>;

/// `SettableFuture<byte[]>`: completed from another thread; `Err` is a failed future, the string
/// its message (`"cancelled"` stands for a cancelled one: `InterruptedException`).
#[derive(Clone)]
struct Settable {
    sender: Arc<Mutex<Option<futures::channel::oneshot::Sender<Outcome>>>>,
    receiver: Shared<futures::channel::oneshot::Receiver<Outcome>>,
}

impl Settable {
    fn new() -> Settable {
        let (sender, receiver) = futures::channel::oneshot::channel();
        Settable {
            sender: Arc::new(Mutex::new(Some(sender))),
            receiver: receiver.shared(),
        }
    }

    fn immediate(outcome: Outcome) -> Settable {
        let settable = Settable::new();
        settable.set(outcome);
        settable
    }

    fn set(&self, outcome: Outcome) {
        if let Some(sender) = self.sender.lock().take() {
            let _ = sender.send(outcome);
        }
    }

    fn is_done(&self) -> bool {
        self.sender.lock().is_none()
    }
}

/// The mocked `GrpcCacheClient`.
struct FakeSplitter {
    verify: AtomicBool,
    /// `splitBlob(..)`: `None` is Java's `null` future.
    split: Mutex<Option<SplitBlobResponse>>,
}

impl ChunkSplitter for FakeSplitter {
    fn should_verify_downloads(&self) -> bool {
        self.verify.load(Ordering::SeqCst)
    }

    fn split_blob<'a>(
        &'a self,
        _context: &'a RemoteActionExecutionContext<'_>,
        _digest: &Digest,
        _chunking_function: chunking_function::Value,
    ) -> Option<BoxFuture<'a, io::Result<SplitBlobResponse>>> {
        let response = self.split.lock().clone()?;
        Some(Box::pin(async move { Ok(response) }))
    }
}

/// The mocked `CombinedCache`: one `Settable` per chunk digest, a call count per digest, and
/// whether a download was dropped before it completed (`isCancelled()`).
#[derive(Default)]
struct FakeSource {
    answers: Mutex<HashMap<String, Settable>>,
    calls: Mutex<HashMap<String, usize>>,
    cancelled: Mutex<Vec<String>>,
    /// A failure that the chunk source reports as a missing blob.
    not_found: Mutex<Vec<String>>,
}

impl FakeSource {
    fn when(&self, digest: &Digest, settable: Settable) {
        self.answers.lock().insert(digest.hash.clone(), settable);
    }

    fn calls(&self, digest: &Digest) -> usize {
        self.calls.lock().get(&digest.hash).copied().unwrap_or(0)
    }
}

/// Drop guard: records the digest as cancelled if the download did not complete.
struct CancelGuard<'a> {
    source: &'a FakeSource,
    hash: String,
    done: bool,
}

impl Drop for CancelGuard<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.source.cancelled.lock().push(self.hash.clone());
        }
    }
}

impl ChunkSource for FakeSource {
    fn download_blob<'a>(
        &'a self,
        _context: &'a RemoteActionExecutionContext<'_>,
        digest: &'a Digest,
    ) -> BoxFuture<'a, io::Result<Vec<u8>>> {
        *self.calls.lock().entry(digest.hash.clone()).or_default() += 1;
        let settable = self
            .answers
            .lock()
            .get(&digest.hash)
            .cloned()
            .expect("unexpected download");
        let not_found = self.not_found.lock().contains(&digest.hash);
        let digest = digest.clone();
        Box::pin(async move {
            let mut guard = CancelGuard {
                source: self,
                hash: digest.hash.clone(),
                done: false,
            };
            let outcome = settable.receiver.await.expect("settable dropped");
            guard.done = true;
            match outcome {
                Ok(data) => Ok(data),
                Err(_) if not_found => Err(CacheNotFoundException::new(digest).into()),
                Err(message) if message == "cancelled" => Err(errors::interrupted()),
                Err(message) => Err(io::Error::other(message)),
            }
        })
    }
}

struct Fixture {
    splitter: FakeSplitter,
    source: FakeSource,
}

impl Fixture {
    // Java: setUp
    fn new() -> Arc<Fixture> {
        Arc::new(Fixture {
            splitter: FakeSplitter {
                verify: AtomicBool::new(true),
                split: Mutex::new(None),
            },
            source: FakeSource::default(),
        })
    }

    fn split_into(&self, chunks: &[&Digest]) {
        *self.splitter.split.lock() = Some(SplitBlobResponse {
            chunk_digests: chunks.iter().map(|d| (*d).clone()).collect(),
            ..Default::default()
        });
    }

    fn download(&self, blob_digest: &Digest) -> (io::Result<()>, Vec<u8>) {
        let config = ChunkingConfig::FastCdc(FastCdcChunkingConfig::defaults());
        let downloader =
            ChunkedBlobDownloader::new(&self.splitter, &self.source, &config, digest_util());
        let context = RemoteActionExecutionContext::create(RequestMetadata::default());
        let mut out = Vec::new();
        let result = block_on(downloader.download_chunked(&context, blob_digest, &mut out));
        (result, out)
    }

    /// `Thread.ofVirtual().unstarted(() -> downloader.downloadChunked(..)).start()`.
    fn download_in_background(
        self: &Arc<Self>,
        blob_digest: &Digest,
    ) -> std::thread::JoinHandle<(io::Result<()>, Vec<u8>)> {
        let fixture = self.clone();
        let blob_digest = blob_digest.clone();
        std::thread::spawn(move || fixture.download(&blob_digest))
    }
}

/// `CountDownLatch.await(timeout)` over a condition.
fn await_until(timeout: Duration, condition: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if condition() {
            return true;
        }
        if start.elapsed() > timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn join(handle: std::thread::JoinHandle<(io::Result<()>, Vec<u8>)>) -> (io::Result<()>, Vec<u8>) {
    assert!(await_until(Duration::from_secs(1), || handle.is_finished()));
    handle.join().unwrap()
}

#[test]
fn download_chunked_split_blob_returns_null_throws_blob_not_splittable() {
    let f = Fixture::new();
    let blob_digest = compute(&[1, 2, 3]);
    let (result, _) = f.download(&blob_digest);
    assert!(BlobNotSplittableException::is(&result.unwrap_err()));
}

#[test]
fn download_chunked_server_returns_no_chunks_throws_blob_not_splittable() {
    let f = Fixture::new();
    let blob_digest = compute(&[1, 2, 3]);
    f.split_into(&[]);
    let (result, _) = f.download(&blob_digest);
    assert!(BlobNotSplittableException::is(&result.unwrap_err()));
}

#[test]
fn download_chunked_chunk_missing_propagates_cache_not_found() {
    let f = Fixture::new();
    let chunk1 = compute(&[1, 2, 3]);
    let chunk2 = compute(&[4, 5, 6]);
    let blob_digest = compute(&[1, 2, 3, 4, 5, 6]);
    f.split_into(&[&chunk1, &chunk2]);
    f.source
        .when(&chunk1, Settable::immediate(Ok(vec![1, 2, 3])));
    f.source.not_found.lock().push(chunk2.hash.clone());
    f.source
        .when(&chunk2, Settable::immediate(Err("not found".into())));

    // A missing chunk is a missing blob, not an invitation to retry the download differently.
    let (result, _) = f.download(&blob_digest);
    let e = result.unwrap_err();
    assert!(
        errors::downcast_ref::<CacheNotFoundException>(&e).is_some(),
        "{e:?}"
    );
}

#[test]
fn download_chunked_single_chunk_downloads_and_reassembles() {
    let f = Fixture::new();
    let chunk_data = vec![1, 2, 3, 4, 5];
    let chunk_digest = compute(&chunk_data);
    f.split_into(&[&chunk_digest]);
    f.source
        .when(&chunk_digest, Settable::immediate(Ok(chunk_data.clone())));
    let (result, out) = f.download(&chunk_digest);
    result.unwrap();
    assert_eq!(out, chunk_data);
}

#[test]
fn download_chunked_multiple_chunks_downloads_and_reassembles_in_order() {
    let f = Fixture::new();
    let chunks = [vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
    let digests: Vec<Digest> = chunks.iter().map(|c| compute(c)).collect();
    let blob_digest = compute(&[1, 2, 3, 4, 5, 6, 7, 8, 9]);
    f.split_into(&digests.iter().collect::<Vec<_>>());
    for (digest, data) in digests.iter().zip(&chunks) {
        f.source.when(digest, Settable::immediate(Ok(data.clone())));
    }
    let (result, out) = f.download(&blob_digest);
    result.unwrap();
    assert_eq!(out, vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
    for digest in &digests {
        assert_eq!(f.source.calls(digest), 1);
    }
}

#[test]
fn download_chunked_window_refills_after_one_chunk_completes() {
    let f = Fixture::new();
    let n = MAX_IN_FLIGHT_CHUNK_DOWNLOADS + 1;
    let expected_data: Vec<u8> = (0..n).map(|i| (i + 1) as u8).collect();
    let chunk_digests: Vec<Digest> = expected_data.iter().map(|b| compute(&[*b])).collect();
    let chunk_futures: Vec<Settable> = (0..n).map(|_| Settable::new()).collect();
    f.split_into(&chunk_digests.iter().collect::<Vec<_>>());
    for (digest, settable) in chunk_digests.iter().zip(&chunk_futures) {
        f.source.when(digest, settable.clone());
    }
    let blob_digest = compute(&expected_data);

    let handle = f.download_in_background(&blob_digest);

    let first_window_requested = || {
        chunk_digests[..MAX_IN_FLIGHT_CHUNK_DOWNLOADS]
            .iter()
            .all(|d| f.source.calls(d) == 1)
    };
    let overflow_chunk_requested = || {
        f.source
            .calls(&chunk_digests[MAX_IN_FLIGHT_CHUNK_DOWNLOADS])
            == 1
    };
    assert!(await_until(Duration::from_secs(1), first_window_requested));
    assert!(!await_until(
        Duration::from_millis(100),
        overflow_chunk_requested
    ));
    chunk_futures[0].set(Ok(vec![expected_data[0]]));
    assert!(await_until(
        Duration::from_secs(1),
        overflow_chunk_requested
    ));
    for (i, settable) in chunk_futures.iter().enumerate() {
        if !settable.is_done() {
            settable.set(Ok(vec![expected_data[i]]));
        }
    }
    let (result, out) = join(handle);
    result.unwrap();
    assert_eq!(out, expected_data);
}

#[test]
fn download_chunked_duplicate_in_flight_chunks_reuses_download() {
    let f = Fixture::new();
    let chunk_data = vec![1, 2, 3];
    let chunk_digest = compute(&chunk_data);
    let blob_digest = compute(&[1, 2, 3, 1, 2, 3]);
    f.split_into(&[&chunk_digest, &chunk_digest]);
    let chunk_future = Settable::new();
    f.source.when(&chunk_digest, chunk_future.clone());

    let handle = f.download_in_background(&blob_digest);
    chunk_future.set(Ok(chunk_data));
    let (result, out) = join(handle);
    result.unwrap();
    assert_eq!(out, vec![1, 2, 3, 1, 2, 3]);
    assert_eq!(f.source.calls(&chunk_digest), 1);
}

#[test]
fn download_chunked_long_duplicate_run_resumes_after_drain() {
    let f = Fixture::new();
    let first = compute(&[1]);
    let duplicate = compute(&[2]);
    let last = compute(&[3]);
    let mut blob_data = vec![1u8];
    let mut split = vec![&first];
    for _ in 1..MAX_IN_FLIGHT_CHUNK_DOWNLOADS {
        blob_data.push(2);
        split.push(&duplicate);
    }
    blob_data.push(3);
    split.push(&last);
    let blob_digest = compute(&blob_data);
    f.split_into(&split);
    let first_future = Settable::new();
    let duplicate_future = Settable::new();
    let last_future = Settable::new();
    f.source.when(&first, first_future.clone());
    f.source.when(&duplicate, duplicate_future.clone());
    f.source.when(&last, last_future.clone());

    let handle = f.download_in_background(&blob_digest);

    let initial_downloads_requested =
        || f.source.calls(&first) == 1 && f.source.calls(&duplicate) == 1;
    let final_chunk_requested = || f.source.calls(&last) == 1;
    assert!(await_until(
        Duration::from_secs(1),
        initial_downloads_requested
    ));
    assert!(!await_until(
        Duration::from_millis(100),
        final_chunk_requested
    ));
    duplicate_future.set(Ok(vec![2]));
    assert!(!await_until(
        Duration::from_millis(100),
        final_chunk_requested
    ));
    first_future.set(Ok(vec![1]));
    assert!(await_until(Duration::from_secs(1), final_chunk_requested));
    last_future.set(Ok(vec![3]));
    let (result, out) = join(handle);
    result.unwrap();
    assert_eq!(out, blob_data);
}

#[test]
fn download_chunked_empty_chunk_list_produces_empty_output() {
    let f = Fixture::new();
    let blob_digest = compute(&[]);
    f.split_into(&[]);
    let (result, out) = f.download(&blob_digest);
    result.unwrap();
    assert!(out.is_empty());
}

#[test]
fn download_chunked_chunk_fails_throws_io_exception() {
    let f = Fixture::new();
    let chunk1 = compute(&[1, 2, 3]);
    let chunk2 = compute(&[4, 5, 6]);
    let blob_digest = compute(&[1, 2, 3, 4, 5, 6]);
    f.split_into(&[&chunk1, &chunk2]);
    f.source
        .when(&chunk1, Settable::immediate(Ok(vec![1, 2, 3])));
    f.source
        .when(&chunk2, Settable::immediate(Err("connection reset".into())));
    let (result, out) = f.download(&blob_digest);
    assert!(result.is_err());
    // Chunks are written as they arrive, so a failed download leaves a prefix of the blob behind.
    // Callers must not restart the download into the same stream.
    assert_eq!(out, vec![1, 2, 3]);
}

#[test]
fn download_chunked_blob_digest_mismatch_throws_output_digest_mismatch() {
    let f = Fixture::new();
    let chunk_data = vec![1, 2, 3];
    let chunk_digest = compute(&chunk_data);
    let blob_digest = compute(&[4, 5, 6]);
    f.split_into(&[&chunk_digest]);
    f.source
        .when(&chunk_digest, Settable::immediate(Ok(chunk_data)));
    let (result, _) = f.download(&blob_digest);
    let e = result.unwrap_err();
    assert!(
        errors::downcast_ref::<OutputDigestMismatchException>(&e).is_some(),
        "{e:?}"
    );
    let message = e.to_string();
    assert!(message.contains(&blob_digest.hash), "{message}");
    assert!(message.contains(&chunk_digest.hash), "{message}");
}

#[test]
fn download_chunked_blob_digest_mismatch_verification_disabled_succeeds() {
    let f = Fixture::new();
    f.splitter.verify.store(false, Ordering::SeqCst);
    let chunk_data = vec![1, 2, 3];
    let chunk_digest = compute(&chunk_data);
    let blob_digest = compute(&[4, 5, 6]);
    f.split_into(&[&chunk_digest]);
    f.source
        .when(&chunk_digest, Settable::immediate(Ok(chunk_data.clone())));
    let (result, out) = f.download(&blob_digest);
    result.unwrap();
    assert_eq!(out, chunk_data);
}

#[test]
fn download_chunked_cancelled_chunk_throws_interrupted_exception() {
    let f = Fixture::new();
    let chunk_digest = compute(&[1, 2, 3]);
    f.split_into(&[&chunk_digest]);
    f.source
        .when(&chunk_digest, Settable::immediate(Err("cancelled".into())));
    let (result, _) = f.download(&chunk_digest);
    assert!(errors::is_interrupted(&result.unwrap_err()));
}

#[test]
fn download_chunked_chunk_fails_cancels_other_in_flight_downloads() {
    let f = Fixture::new();
    let chunk1 = compute(&[1, 2, 3]);
    let chunk2 = compute(&[4, 5, 6]);
    let blob_digest = compute(&[1, 2, 3, 4, 5, 6]);
    f.split_into(&[&chunk1, &chunk2]);
    let failed_download = Settable::new();
    let cancelled_download = Settable::new();
    f.source.when(&chunk1, failed_download.clone());
    f.source.when(&chunk2, cancelled_download.clone());

    let handle = f.download_in_background(&blob_digest);
    assert!(await_until(Duration::from_secs(1), || {
        f.source.calls(&chunk1) == 1 && f.source.calls(&chunk2) == 1
    }));
    failed_download.set(Err("connection reset".into()));
    let (result, _) = join(handle);
    assert!(result.is_err());
    assert!(f.source.cancelled.lock().contains(&chunk2.hash));
}
