//! Port of `BuildEventArtifactUploader` and `LocalFilesArtifactUploader`.

use std::collections::HashMap;
use std::sync::mpsc;

use blaze_vfs::Path;

use super::build_event::{LocalFile, LocalFileType};
use super::build_event_context::{FileUriPathConverter, PathConverter};

/// Why a [`PathConverterFuture`] completed without a [`PathConverter`]: Java's
/// `CancellationException` or the cause of its `ExecutionException`.
#[derive(Debug)]
pub enum UploadFailure {
    /// The future was cancelled (`Futures.immediateCancelledFuture()`, or the uploader dropped
    /// its promise without completing it).
    Cancelled,
    /// The future failed with this exception (an `IOException` in every Java uploader).
    Failed(std::io::Error),
}

impl UploadFailure {
    /// `Throwable.getMessage()` of the exception `Future.get()` throws: `"Task was cancelled."`
    /// for a `CancellationException`; for an `ExecutionException` its message, which is the
    /// cause's `toString()`.
    // PORT: Throwable.toString of the cause: the uploader's [`JavaThrowable`] when it gave one,
    // else an IOException ("java.io.IOException: <message>").
    pub fn get_message(&self) -> String {
        match self {
            UploadFailure::Cancelled => "Task was cancelled.".to_string(),
            UploadFailure::Failed(e) => {
                match e.get_ref().and_then(|r| r.downcast_ref::<JavaThrowable>()) {
                    Some(java) => java.to_string.clone(),
                    None => format!("java.io.IOException: {e}"),
                }
            }
        }
    }
}

/// The payload of an `io::Error` an uploader fails with when it knows the Java class of the
/// exception: its `Throwable.toString()` (`<class name>: <message>`); it displays the message
/// after the class name.
#[derive(Debug, Clone)]
pub struct JavaThrowable {
    /// `Throwable.toString()`.
    pub to_string: String,
}

impl JavaThrowable {
    /// The exception whose `toString()` is `to_string`.
    pub fn new(to_string: String) -> JavaThrowable {
        JavaThrowable { to_string }
    }
}

impl std::fmt::Display for JavaThrowable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.to_string.split_once(": ") {
            Some((_, message)) => f.write_str(message),
            None => Ok(()),
        }
    }
}

impl std::error::Error for JavaThrowable {}

type UploadResult = Result<Box<dyn PathConverter>, UploadFailure>;

/// `ListenableFuture<PathConverter>`: the result of [`BuildEventArtifactUploader::upload`].
/// Either already complete (the local uploader) or completed later by the uploader through the
/// [`PathConverterPromise`] of [`PathConverterFuture::create`] (`SettableFuture`).
pub struct PathConverterFuture {
    inner: FutureInner,
}

enum FutureInner {
    Done(UploadResult),
    Pending(mpsc::Receiver<UploadResult>),
}

impl PathConverterFuture {
    /// `Futures.immediateFuture(converter)`.
    pub fn immediate(converter: Box<dyn PathConverter>) -> PathConverterFuture {
        PathConverterFuture {
            inner: FutureInner::Done(Ok(converter)),
        }
    }

    /// `Futures.immediateCancelledFuture()`.
    pub fn immediate_cancelled() -> PathConverterFuture {
        PathConverterFuture {
            inner: FutureInner::Done(Err(UploadFailure::Cancelled)),
        }
    }

    /// `Futures.immediateFailedFuture(e)`.
    pub fn immediate_failed(e: std::io::Error) -> PathConverterFuture {
        PathConverterFuture {
            inner: FutureInner::Done(Err(UploadFailure::Failed(e))),
        }
    }

    /// `SettableFuture.create()`: a pending future and the promise that completes it.
    pub fn create() -> (PathConverterPromise, PathConverterFuture) {
        let (sender, receiver) = mpsc::sync_channel(1);
        (
            PathConverterPromise { sender },
            PathConverterFuture {
                inner: FutureInner::Pending(receiver),
            },
        )
    }

    /// `Future.isDone()`: whether `get` returns without waiting.
    pub fn is_done(&self) -> bool {
        matches!(self.inner, FutureInner::Done(_))
    }

    /// `Future.get()`: waits for the upload. A promise dropped without a value is a cancelled
    /// future.
    pub fn get(self) -> Result<Box<dyn PathConverter>, UploadFailure> {
        match self.inner {
            FutureInner::Done(result) => result,
            FutureInner::Pending(receiver) => match receiver.recv() {
                Ok(result) => result,
                Err(mpsc::RecvError) => Err(UploadFailure::Cancelled),
            },
        }
    }
}

/// The completing side of a [`PathConverterFuture`] (`SettableFuture.set` / `setException`).
pub struct PathConverterPromise {
    sender: mpsc::SyncSender<UploadResult>,
}

impl PathConverterPromise {
    /// `SettableFuture.set(converter)`.
    pub fn set(self, converter: Box<dyn PathConverter>) {
        // The future may have been dropped; then nobody waits for the value.
        let _ = self.sender.send(Ok(converter));
    }

    /// `SettableFuture.setException(e)`.
    pub fn set_exception(self, e: std::io::Error) {
        let _ = self.sender.send(Err(UploadFailure::Failed(e)));
    }
}

/// Uploads artifacts referenced by the Build Event Protocol (BEP).
///
/// Java's uploader is `ReferenceCounted`; [`BuildEventArtifactUploader::retain`] and
/// [`BuildEventArtifactUploader::release`] are its `retain()` / `release()` (no-ops for an
/// uploader that holds no resources: `LocalFilesArtifactUploader.deallocate` does nothing).
// Java: com/google/devtools/build/lib/buildeventstream/BuildEventArtifactUploader.java:34 BuildEventArtifactUploader
pub trait BuildEventArtifactUploader: Send + Sync {
    /// Asynchronously uploads a set of files referenced by the protobuf representation of a
    /// build event. This method is expected to return quickly.
    ///
    /// This method must not throw any exceptions.
    ///
    /// Returns a future to a [`PathConverter`] that must provide a name for each uploaded file
    /// as it should appear in the BEP.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventArtifactUploader.java:44 BuildEventArtifactUploader.upload
    fn upload(&self, files: HashMap<Path, LocalFile>) -> PathConverterFuture;

    /// Return true if the upload may be "slow". Examples of slowness include writes to remote
    /// storage.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventArtifactUploader.java:85 BuildEventArtifactUploader.mayBeSlow
    fn may_be_slow(&self) -> bool;

    /// Returns a [`PathConverter`] for the uploaded files, or a failed future when the upload
    /// failed (the first `LocalFile` per path wins).
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventArtifactUploader.java:91 BuildEventArtifactUploader.uploadReferencedLocalFiles
    fn upload_referenced_local_files(&self, local_files: &[LocalFile]) -> PathConverterFuture {
        let mut local_file_map: HashMap<Path, LocalFile> =
            HashMap::with_capacity(local_files.len());
        for local_file in local_files {
            // It is possible for targets to have duplicate artifacts (same path but different
            // owners) in their output groups. Since they didn't trigger an artifact conflict
            // they are the same file, so just skip either one
            local_file_map
                .entry(local_file.path.clone())
                .or_insert_with(|| local_file.clone());
        }
        self.upload(local_file_map)
    }

    /// `ReferenceCounted.retain()`.
    // Java: io/netty/util/ReferenceCounted.java ReferenceCounted.retain
    fn retain(&self) {}

    /// `ReferenceCounted.release()`: the last release frees the uploader's resources.
    // Java: io/netty/util/ReferenceCounted.java ReferenceCounted.release
    fn release(&self) {}
}

/// An uploader that simply turns paths into local file URIs.
// Java: com/google/devtools/build/lib/buildeventstream/LocalFilesArtifactUploader.java:29 LocalFilesArtifactUploader
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalFilesArtifactUploader;

impl BuildEventArtifactUploader for LocalFilesArtifactUploader {
    // Java: com/google/devtools/build/lib/buildeventstream/LocalFilesArtifactUploader.java:34 LocalFilesArtifactUploader.upload
    fn upload(&self, files: HashMap<Path, LocalFile>) -> PathConverterFuture {
        PathConverterFuture::immediate(Box::new(PathConverterImpl { paths: files }))
    }

    // Java: com/google/devtools/build/lib/buildeventstream/LocalFilesArtifactUploader.java:49 LocalFilesArtifactUploader.mayBeSlow
    fn may_be_slow(&self) -> bool {
        false
    }
}

// Java: com/google/devtools/build/lib/buildeventstream/LocalFilesArtifactUploader.java:53 LocalFilesArtifactUploader.PathConverterImpl
struct PathConverterImpl {
    paths: HashMap<Path, LocalFile>,
}

impl PathConverter for PathConverterImpl {
    // Java: com/google/devtools/build/lib/buildeventstream/LocalFilesArtifactUploader.java:62 PathConverterImpl.apply
    fn apply(&self, path: &Path) -> Option<String> {
        let local_file = self.paths.get(path)?;
        let type_ = local_file.type_;
        if type_ == LocalFileType::OutputDirectory || type_ == LocalFileType::OutputSymlink {
            return None;
        }
        FileUriPathConverter.apply(path)
    }
}
