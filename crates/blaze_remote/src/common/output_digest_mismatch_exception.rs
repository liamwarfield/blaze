//! Port of `lib/remote/common/OutputDigestMismatchException.java`.

use std::fmt;
use std::io;

use blaze_vfs::Path;

use crate::reapi::Digest;

/// An exception to indicate the digest of downloaded output does not match the expected value.
// Java: com/google/devtools/build/lib/remote/common/OutputDigestMismatchException.java:23 OutputDigestMismatchException
#[derive(Debug, Clone)]
pub struct OutputDigestMismatchException {
    expected: Digest,
    actual: Option<Digest>,
    received_size_bytes: i64,
    local_path: Option<Path>,
    output_path: Option<String>,
}

impl OutputDigestMismatchException {
    /// `new OutputDigestMismatchException(expected, actual)`.
    // Java: com/google/devtools/build/lib/remote/common/OutputDigestMismatchException.java:31 OutputDigestMismatchException.<init>
    pub fn new(expected: Digest, actual: Digest) -> OutputDigestMismatchException {
        let received_size_bytes = actual.size_bytes;
        OutputDigestMismatchException {
            expected,
            actual: Some(actual),
            received_size_bytes,
            local_path: None,
            output_path: None,
        }
    }

    /// `new OutputDigestMismatchException(expected, receivedSizeBytes)`.
    // Java: com/google/devtools/build/lib/remote/common/OutputDigestMismatchException.java:37 OutputDigestMismatchException.<init>
    pub fn with_received_size(expected: Digest, received_size_bytes: i64) -> OutputDigestMismatchException {
        OutputDigestMismatchException {
            expected,
            actual: None,
            received_size_bytes,
            local_path: None,
            output_path: None,
        }
    }

    /// `setOutputPath(outputPath)`.
    pub fn set_output_path(&mut self, output_path: &str) {
        self.output_path = Some(output_path.to_string());
    }

    /// `getOutputPath()`.
    pub fn get_output_path(&self) -> Option<&str> {
        self.output_path.as_deref()
    }

    /// `getLocalPath()`.
    pub fn get_local_path(&self) -> Option<&Path> {
        self.local_path.as_ref()
    }

    /// `setLocalPath(localPath)`.
    pub fn set_local_path(&mut self, local_path: Path) {
        self.local_path = Some(local_path);
    }

    /// `getMessage()`.
    // Java: com/google/devtools/build/lib/remote/common/OutputDigestMismatchException.java:60 OutputDigestMismatchException.getMessage
    pub fn get_message(&self) -> String {
        // `String.format("%s", null)` prints "null".
        let output_path = self.output_path.as_deref().unwrap_or("null");
        match &self.actual {
            None => format!(
                "Output {} download failed: Expected digest '{}/{}', but received {} bytes.",
                output_path, self.expected.hash, self.expected.size_bytes, self.received_size_bytes
            ),
            Some(actual) => format!(
                "Output {} download failed: Expected digest '{}/{}' does not match received digest '{}/{}'.",
                output_path,
                self.expected.hash,
                self.expected.size_bytes,
                actual.hash,
                actual.size_bytes
            ),
        }
    }
}

impl fmt::Display for OutputDigestMismatchException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_message())
    }
}

impl std::error::Error for OutputDigestMismatchException {}

impl From<OutputDigestMismatchException> for io::Error {
    fn from(e: OutputDigestMismatchException) -> io::Error {
        io::Error::other(e)
    }
}
