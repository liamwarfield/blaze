//! Port of `lib/remote/common/BlobNotSplittableException.java`.

use std::error::Error;
use std::fmt;
use std::io;

use crate::reapi::Digest;

/// Indicates that the server cannot describe a blob as a sequence of chunks, either because it
/// does not implement `SplitBlob` or because it has no chunks for this particular blob.
///
/// This says nothing about whether the blob itself is available: the caller is expected to fall
/// back to downloading it as a whole. It is only thrown before any chunk data has been produced,
/// so that such a fallback can safely start from the beginning of the blob. Carried in an
/// `io::Error` (an `IOException` subclass).
// Java: com/google/devtools/build/lib/remote/common/BlobNotSplittableException.java:28 BlobNotSplittableException
#[derive(Debug, Clone)]
pub struct BlobNotSplittableException {
    blob_digest: Digest,
}

impl BlobNotSplittableException {
    // Java: com/google/devtools/build/lib/remote/common/BlobNotSplittableException.java:31 BlobNotSplittableException.<init>
    pub fn new(blob_digest: Digest) -> BlobNotSplittableException {
        BlobNotSplittableException { blob_digest }
    }

    // Java: com/google/devtools/build/lib/remote/common/BlobNotSplittableException.java:35 BlobNotSplittableException.getBlobDigest
    pub fn get_blob_digest(&self) -> &Digest {
        &self.blob_digest
    }

    /// `e instanceof BlobNotSplittableException`.
    pub fn is(e: &io::Error) -> bool {
        e.get_ref()
            .is_some_and(|inner| inner.downcast_ref::<BlobNotSplittableException>().is_some())
    }
}

impl fmt::Display for BlobNotSplittableException {
    // Java: com/google/devtools/build/lib/remote/common/BlobNotSplittableException.java:40 BlobNotSplittableException.getMessage
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Not splittable into chunks: {}/{}",
            self.blob_digest.hash, self.blob_digest.size_bytes
        )
    }
}

impl Error for BlobNotSplittableException {}

impl From<BlobNotSplittableException> for io::Error {
    fn from(e: BlobNotSplittableException) -> io::Error {
        io::Error::other(e)
    }
}
