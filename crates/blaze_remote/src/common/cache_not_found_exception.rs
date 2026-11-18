//! Port of `lib/remote/common/CacheNotFoundException.java`.

use std::fmt;
use std::io;

use blaze_vfs::PathFragment;

use crate::reapi::Digest;

/// An exception to indicate cache misses.
// Java: com/google/devtools/build/lib/remote/common/CacheNotFoundException.java:26 CacheNotFoundException
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheNotFoundException {
    missing_digest: Digest,
    exec_path: Option<PathFragment>,
    filename: Option<String>,
}

impl CacheNotFoundException {
    /// `new CacheNotFoundException(missingDigest)`.
    // Java: com/google/devtools/build/lib/remote/common/CacheNotFoundException.java:31 CacheNotFoundException.<init>
    pub fn new(missing_digest: Digest) -> CacheNotFoundException {
        CacheNotFoundException {
            missing_digest,
            exec_path: None,
            filename: None,
        }
    }

    /// `new CacheNotFoundException(missingDigest, execPath)`.
    // Java: com/google/devtools/build/lib/remote/common/CacheNotFoundException.java:35 CacheNotFoundException.<init>
    pub fn with_exec_path(
        missing_digest: Digest,
        exec_path: PathFragment,
    ) -> CacheNotFoundException {
        CacheNotFoundException {
            missing_digest,
            exec_path: Some(exec_path),
            filename: None,
        }
    }

    /// `new CacheNotFoundException(missingDigest, filename)`.
    // Java: com/google/devtools/build/lib/remote/common/CacheNotFoundException.java:40 CacheNotFoundException.<init>
    pub fn with_filename(missing_digest: Digest, filename: &str) -> CacheNotFoundException {
        CacheNotFoundException {
            missing_digest,
            exec_path: None,
            filename: Some(filename.to_string()),
        }
    }

    /// The exec path of the artifact that was not found in the cache if the missing cache entry
    /// corresponds to one.
    // Java: com/google/devtools/build/lib/remote/common/CacheNotFoundException.java:47 CacheNotFoundException.setExecPath
    pub fn set_exec_path(&mut self, exec_path: PathFragment) {
        self.exec_path = Some(exec_path);
    }

    /// A human-readable filename only used in error messages.
    // Java: com/google/devtools/build/lib/remote/common/CacheNotFoundException.java:52 CacheNotFoundException.setFilename
    pub fn set_filename(&mut self, filename: &str) {
        self.filename = Some(filename.to_string());
    }

    /// `getMissingDigest()`.
    pub fn get_missing_digest(&self) -> &Digest {
        &self.missing_digest
    }

    /// `getExecPath()`.
    pub fn get_exec_path(&self) -> Option<&PathFragment> {
        self.exec_path.as_ref()
    }

    /// `getFilename()`.
    pub fn get_filename(&self) -> Option<&str> {
        self.filename.as_deref()
    }

    /// `getMessage()`.
    // Java: com/google/devtools/build/lib/remote/common/CacheNotFoundException.java:70 CacheNotFoundException.getMessage
    pub fn get_message(&self) -> String {
        let mut message = format!(
            "Missing digest: {}/{}",
            self.missing_digest.hash, self.missing_digest.size_bytes
        );
        if self.exec_path.is_some() || self.filename.is_some() {
            // Prefer filename over execPath as it contains strictly more information.
            message += " for ";
            match (&self.filename, &self.exec_path) {
                (Some(filename), _) => message += filename,
                (None, Some(exec_path)) => message += exec_path.get_path_string(),
                (None, None) => unreachable!(),
            }
        }
        message
    }
}

impl fmt::Display for CacheNotFoundException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_message())
    }
}

impl std::error::Error for CacheNotFoundException {}

impl From<CacheNotFoundException> for io::Error {
    fn from(e: CacheNotFoundException) -> io::Error {
        io::Error::other(e)
    }
}
