//! Port of `lib/remote/ChunkedBlobUploaderTest.java`.
//!
//! PORT: Java mocks `GrpcCacheClient` and `CombinedCache` with Mockito; the uploader's seam here
//! is the trait `ChunkedUploadTarget`, implemented by [`FakeTarget`]. A `SettableFuture<Void>` is
//! a [`Settable`]; Java's upload thread is a thread running `block_on`. Java's `InMemoryFileSystem`
//! is a temporary directory on the real one; its mocked `Path` (a stream that changes between
//! calls) is the file itself, truncated between chunking and uploading.

#[path = "util/java_random.rs"]
mod java_random;

use std::collections::{HashMap, HashSet};
use std::io::{self, Read};
use std::sync::Arc;
use std::time::{Duration, Instant};

use blaze_remote::chunked_blob_uploader::{ChunkedBlobUploader, ChunkedUploadTarget};
use blaze_remote::chunking::{
    ChunkingConfig, ContentDefinedChunker, FastCdcChunker, FastCdcChunkingConfig,
};
use blaze_remote::common::errors;
use blaze_remote::common::{Blob, RemoteActionExecutionContext};
use blaze_remote::reapi::{Digest, RequestMetadata, chunking_function};
use blaze_remote::runtime::block_on;
use blaze_remote::util::DigestUtil;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};
use futures::future::{BoxFuture, FutureExt, Shared};
use indexmap::IndexSet;
use java_random::random_bytes;
use parking_lot::Mutex;

const MAX_IN_FLIGHT_CHUNK_UPLOADS: usize = 16;

fn digest_util() -> DigestUtil {
    DigestUtil::new(DigestHashFunction::Sha256)
}

type Outcome = Result<(), String>;

/// `SettableFuture<Void>`; `Err("cancelled")` is a cancelled one (`InterruptedException`).
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

/// The mocked `GrpcCacheClient` and `CombinedCache`.
#[derive(Default)]
struct FakeTarget {
    /// `findMissingDigests`: `None` answers with every digest it is asked for.
    missing: Mutex<Option<IndexSet<Digest>>>,
    /// The digests `findMissingDigests` was asked for (`digestsCaptor`).
    find_missing_calls: Mutex<Vec<Vec<Digest>>>,
    /// Called when `findMissingDigests` is asked (to change the file under the uploader).
    on_find_missing: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    splice_calls: Mutex<Vec<(Digest, Vec<Digest>)>>,
    /// Per chunk hash: the answer of `uploadBlob`; absent: an immediate success.
    upload_answers: Mutex<HashMap<String, Settable>>,
    /// Whether `uploadBlob` reads the blob (`thenAnswer` reading `blob.get()`).
    read_blobs: bool,
    /// (digest, force, data read or empty) per `uploadBlob` call.
    upload_calls: Mutex<Vec<(Digest, bool, Vec<u8>)>>,
    cancelled: Mutex<Vec<String>>,
}

struct CancelGuard<'a> {
    target: &'a FakeTarget,
    hash: String,
    done: bool,
}

impl Drop for CancelGuard<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.target.cancelled.lock().push(self.hash.clone());
        }
    }
}

impl ChunkedUploadTarget for FakeTarget {
    fn find_missing_digests<'a>(
        &'a self,
        _context: &'a RemoteActionExecutionContext<'_>,
        digests: &'a [Digest],
    ) -> BoxFuture<'a, io::Result<IndexSet<Digest>>> {
        self.find_missing_calls.lock().push(digests.to_vec());
        if let Some(hook) = &*self.on_find_missing.lock() {
            hook();
        }
        let missing = self
            .missing
            .lock()
            .clone()
            .unwrap_or_else(|| digests.iter().cloned().collect());
        Box::pin(async move { Ok(missing) })
    }

    fn splice_blob<'a>(
        &'a self,
        _context: &'a RemoteActionExecutionContext<'_>,
        blob_digest: &Digest,
        chunk_digests: &[Digest],
        _chunking_function: chunking_function::Value,
    ) -> Option<BoxFuture<'a, io::Result<()>>> {
        self.splice_calls
            .lock()
            .push((blob_digest.clone(), chunk_digests.to_vec()));
        Some(Box::pin(async { Ok(()) }))
    }

    fn upload_blob<'a>(
        &'a self,
        _context: &'a RemoteActionExecutionContext<'_>,
        digest: &'a Digest,
        blob: Arc<dyn Blob>,
        force: bool,
    ) -> BoxFuture<'a, io::Result<()>> {
        let read = if self.read_blobs {
            let mut data = Vec::new();
            match blob.get().and_then(|mut stream| stream.read_to_end(&mut data)) {
                Ok(_) => Ok(data),
                Err(e) => Err(e),
            }
        } else {
            Ok(Vec::new())
        };
        let data = match read {
            Ok(data) => data,
            Err(e) => return Box::pin(async move { Err(e) }),
        };
        self.upload_calls.lock().push((digest.clone(), force, data));
        let settable = self
            .upload_answers
            .lock()
            .get(&digest.hash)
            .cloned()
            .unwrap_or_else(|| Settable::immediate(Ok(())));
        let hash = digest.hash.clone();
        Box::pin(async move {
            let mut guard = CancelGuard {
                target: self,
                hash,
                done: false,
            };
            let outcome = settable.receiver.await.expect("settable dropped");
            guard.done = true;
            match outcome {
                Ok(()) => Ok(()),
                Err(message) if message == "cancelled" => Err(errors::interrupted()),
                Err(message) => Err(io::Error::other(message)),
            }
        })
    }
}

/// `setUp`: a temporary directory for the files, the uploader over a `FastCdcChunkingConfig(1024,
/// 2, 0)`.
struct Fixture {
    target: Arc<FakeTarget>,
    dir: tempfile::TempDir,
    fs: FileSystemRef,
}

impl Fixture {
    fn new(target: FakeTarget) -> Arc<Fixture> {
        Arc::new(Fixture {
            target: Arc::new(target),
            dir: tempfile::tempdir().unwrap(),
            fs: Arc::new(UnixFileSystem::new(DigestHashFunction::Sha256)),
        })
    }

    fn uploader(&self, config: FastCdcChunkingConfig) -> ChunkedBlobUploader {
        ChunkedBlobUploader::new(
            self.target.clone(),
            &ChunkingConfig::FastCdc(config),
            digest_util(),
        )
    }

    fn write_file(&self, name: &str, data: &[u8]) -> Path {
        let path = self.dir.path().join(name);
        std::fs::write(&path, data).unwrap();
        Path::create_from_str(path.to_str().unwrap(), &self.fs)
    }

    fn upload(&self, blob_digest: &Digest, file: &Path, force: bool) -> io::Result<()> {
        let uploader = self.uploader(FastCdcChunkingConfig::new(1024, 2, 0));
        let context = RemoteActionExecutionContext::create(RequestMetadata::default());
        block_on(uploader.upload_chunked(&context, blob_digest, file, force))
    }

    fn upload_in_background(
        self: &Arc<Self>,
        blob_digest: &Digest,
        file: &Path,
    ) -> std::thread::JoinHandle<io::Result<()>> {
        let fixture = self.clone();
        let blob_digest = blob_digest.clone();
        let file = file.clone();
        std::thread::spawn(move || fixture.upload(&blob_digest, &file, false))
    }
}

fn chunks_of(data: &[u8]) -> Vec<Digest> {
    FastCdcChunker::from_config(&FastCdcChunkingConfig::new(1024, 2, 0), digest_util())
        .chunk_to_digests(&mut &data[..])
        .unwrap()
}

fn unique_prefix(chunk_digests: &[Digest], n: usize) -> Vec<Digest> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for digest in chunk_digests {
        if seen.insert(digest.hash.clone()) {
            unique.push(digest.clone());
        }
        if unique.len() == n {
            break;
        }
    }
    unique
}

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

fn upload_calls_for(target: &FakeTarget, digest: &Digest) -> usize {
    target
        .upload_calls
        .lock()
        .iter()
        .filter(|(d, _, _)| d.hash == digest.hash)
        .count()
}

#[test]
fn get_chunking_threshold_returns_configured_value() {
    let f = Fixture::new(FakeTarget::default());
    let uploader = f.uploader(FastCdcChunkingConfig::new(512, 2, 0));
    assert_eq!(uploader.get_chunking_threshold(), 512 * 4);
}

#[test]
fn upload_chunked_all_chunks_missing_uploads_all_chunks() {
    let f = Fixture::new(FakeTarget::default());
    let data = random_bytes(42, 8192);
    let file = f.write_file("test.txt", &data);
    f.upload(&digest_util().compute(&data), &file, false).unwrap();
    let chunk_digests = f.target.find_missing_calls.lock()[0].clone();
    assert!(chunk_digests.len() > 1);
    let total: i64 = chunk_digests.iter().map(|d| d.size_bytes).sum();
    assert_eq!(total, data.len() as i64);
}

#[test]
fn upload_chunked_force_forwards_force_to_chunk_uploads() {
    let f = Fixture::new(FakeTarget::default());
    let data = random_bytes(42, 8192);
    let file = f.write_file("test_force.txt", &data);
    f.upload(&digest_util().compute(&data), &file, true).unwrap();
    let chunk_count = f.target.find_missing_calls.lock()[0].len();
    let forced = f
        .target
        .upload_calls
        .lock()
        .iter()
        .filter(|(_, force, _)| *force)
        .count();
    assert_eq!(forced, chunk_count);
}

#[test]
fn upload_chunked_no_chunks_missing_skips_chunk_upload() {
    let target = FakeTarget::default();
    *target.missing.lock() = Some(IndexSet::new());
    let f = Fixture::new(target);
    let data = random_bytes(42, 8192);
    let file = f.write_file("test.txt", &data);
    let blob_digest = digest_util().compute(&data);
    f.upload(&blob_digest, &file, false).unwrap();
    assert!(f.target.upload_calls.lock().is_empty());
    let splices = f.target.splice_calls.lock();
    assert_eq!(splices.len(), 1);
    assert_eq!(splices[0].0, blob_digest);
}

#[test]
fn upload_chunked_some_chunks_missing_uploads_only_missing_with_correct_data() {
    let data = random_bytes(42, 16384);
    let all_chunk_digests = chunks_of(&data);
    assert!(all_chunk_digests.len() >= 5);
    let mut missing = IndexSet::new();
    let mut expected: HashMap<String, Vec<u8>> = HashMap::new();
    let mut offset = 0usize;
    for (i, digest) in all_chunk_digests.iter().enumerate() {
        let size = digest.size_bytes as usize;
        if i == 0 || i == all_chunk_digests.len() - 1 || i % 2 == 1 {
            missing.insert(digest.clone());
            expected.insert(digest.hash.clone(), data[offset..offset + size].to_vec());
        }
        offset += size;
    }
    let target = FakeTarget {
        read_blobs: true,
        ..Default::default()
    };
    *target.missing.lock() = Some(missing);
    let f = Fixture::new(target);
    let file = f.write_file("test_partial.txt", &data);
    let blob_digest = digest_util().compute(&data);
    f.upload(&blob_digest, &file, false).unwrap();

    let actual: HashMap<String, Vec<u8>> = f
        .target
        .upload_calls
        .lock()
        .iter()
        .map(|(d, _, data)| (d.hash.clone(), data.clone()))
        .collect();
    assert_eq!(actual, expected);
    let splices = f.target.splice_calls.lock();
    assert_eq!(splices.as_slice(), &[(blob_digest, all_chunk_digests)]);
}

#[test]
fn upload_chunked_window_refills_after_one_chunk_completes() {
    let data = random_bytes(42, 262144);
    let chunk_digests = chunks_of(&data);
    let unique = unique_prefix(&chunk_digests, MAX_IN_FLIGHT_CHUNK_UPLOADS + 1);
    assert_eq!(unique.len(), MAX_IN_FLIGHT_CHUNK_UPLOADS + 1);
    let uploads: Vec<Settable> = unique.iter().map(|_| Settable::new()).collect();
    let target = FakeTarget::default();
    *target.missing.lock() = Some(unique.iter().cloned().collect());
    for (digest, settable) in unique.iter().zip(&uploads) {
        target
            .upload_answers
            .lock()
            .insert(digest.hash.clone(), settable.clone());
    }
    let f = Fixture::new(target);
    let file = f.write_file("test_window.txt", &data);
    let blob_digest = digest_util().compute(&data);

    let handle = f.upload_in_background(&blob_digest, &file);

    let first_window_requested = || {
        unique[..MAX_IN_FLIGHT_CHUNK_UPLOADS]
            .iter()
            .all(|d| upload_calls_for(&f.target, d) == 1)
    };
    let overflow_requested =
        || upload_calls_for(&f.target, &unique[MAX_IN_FLIGHT_CHUNK_UPLOADS]) == 1;
    assert!(await_until(Duration::from_secs(1), first_window_requested));
    assert!(!await_until(Duration::from_millis(100), overflow_requested));
    uploads[1].set(Ok(()));
    assert!(await_until(Duration::from_secs(1), overflow_requested));
    for upload in &uploads {
        if !upload.is_done() {
            upload.set(Ok(()));
        }
    }
    assert!(await_until(Duration::from_secs(1), || handle.is_finished()));
    handle.join().unwrap().unwrap();
    let splices = f.target.splice_calls.lock();
    assert_eq!(splices.as_slice(), &[(blob_digest, chunk_digests)]);
}

#[test]
fn upload_chunked_chunk_fails_cancels_other_in_flight_uploads() {
    let data = random_bytes(42, 16384);
    let unique = unique_prefix(&chunks_of(&data), 2);
    assert_eq!(unique.len(), 2);
    let failed_upload = Settable::new();
    let cancelled_upload = Settable::new();
    let target = FakeTarget::default();
    *target.missing.lock() = Some(unique.iter().cloned().collect());
    target
        .upload_answers
        .lock()
        .insert(unique[0].hash.clone(), failed_upload.clone());
    target
        .upload_answers
        .lock()
        .insert(unique[1].hash.clone(), cancelled_upload.clone());
    let f = Fixture::new(target);
    let file = f.write_file("test_failure.txt", &data);
    let blob_digest = digest_util().compute(&data);

    let handle = f.upload_in_background(&blob_digest, &file);
    assert!(await_until(Duration::from_secs(1), || {
        f.target.upload_calls.lock().len() == 2
    }));
    failed_upload.set(Err("upload failed".into()));
    assert!(await_until(Duration::from_secs(1), || handle.is_finished()));
    assert!(handle.join().unwrap().is_err());
    assert!(f.target.cancelled.lock().contains(&unique[1].hash));
    assert!(f.target.splice_calls.lock().is_empty());
}

#[test]
fn upload_chunked_cancelled_upload_throws_interrupted_exception() {
    let data = random_bytes(42, 16384);
    let first_chunk_digest = chunks_of(&data)[0].clone();
    let target = FakeTarget::default();
    *target.missing.lock() = Some([first_chunk_digest.clone()].into_iter().collect());
    target.upload_answers.lock().insert(
        first_chunk_digest.hash.clone(),
        Settable::immediate(Err("cancelled".into())),
    );
    let f = Fixture::new(target);
    let file = f.write_file("test_cancel.txt", &data);
    let e = f
        .upload(&digest_util().compute(&data), &file, false)
        .unwrap_err();
    assert!(errors::is_interrupted(&e), "{e:?}");
    assert!(f.target.splice_calls.lock().is_empty());
}

/// PORT: Java's mocked `Path` counts `getInputStream()` (once: the chunking); here no chunk's
/// stream is opened, which is the same: the fake reads blobs only when asked to.
#[test]
fn upload_chunked_failed_upload_during_pending_chunks_surfaces_before_opening_chunk_stream() {
    let data = random_bytes(42, 16384);
    let chunk_digests = chunks_of(&data);
    assert!(chunk_digests.len() >= 2);
    let target = FakeTarget::default();
    *target.missing.lock() = Some([chunk_digests[0].clone()].into_iter().collect());
    target.upload_answers.lock().insert(
        chunk_digests[0].hash.clone(),
        Settable::immediate(Err("upload failed".into())),
    );
    let f = Fixture::new(target);
    let file = f.write_file("test_pending.txt", &data);
    let handle = f.upload_in_background(&digest_util().compute(&data), &file);
    assert!(await_until(Duration::from_secs(1), || handle.is_finished()));
    assert!(handle.join().unwrap().is_err());
    assert!(f.target.splice_calls.lock().is_empty());
}

#[test]
fn upload_chunked_file_truncated_before_chunk_upload_reports_concurrent_modification() {
    let data = random_bytes(42, 8192);
    let chunk_digests = chunks_of(&data);
    assert!(chunk_digests.len() >= 2);
    let second_chunk_digest = chunk_digests[1].clone();
    let target = FakeTarget {
        read_blobs: true,
        ..Default::default()
    };
    *target.missing.lock() = Some([second_chunk_digest].into_iter().collect());
    let f = Fixture::new(target);
    let file = f.write_file("test_truncated.txt", &data);
    // The file is empty when its chunk is read (Java: a second `getInputStream()` that is empty).
    let path = file.get_path_string().to_string();
    *f.target.on_find_missing.lock() = Some(Box::new(move || {
        std::fs::write(&path, b"").unwrap();
    }));
    let e = f
        .upload(&digest_util().compute(&data), &file, false)
        .unwrap_err();
    assert!(
        e.to_string()
            .contains("file was concurrently modified during upload"),
        "{e}"
    );
    assert!(f.target.splice_calls.lock().is_empty());
}
