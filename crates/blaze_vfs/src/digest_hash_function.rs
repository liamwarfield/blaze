//! Port of `DigestHashFunction` (SHA-256 only).
//!
//! Java registers SHA-1 and SHA-256 (and Bazel adds BLAKE3 elsewhere). Only SHA-256 is ported;
//! the others are not yet needed.

use std::fmt;

use sha2::Digest;

/// A digest function for file contents (Java `DigestHashFunction`).
// Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:39 DigestHashFunction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DigestHashFunction {
    /// SHA-256.
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:69 DigestHashFunction.SHA256
    Sha256,
}

impl DigestHashFunction {
    /// The registered name ("SHA-256").
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:174 DigestHashFunction.toString
    pub fn name(self) -> &'static str {
        match self {
            DigestHashFunction::Sha256 => "SHA-256",
        }
    }

    /// Hashes everything readable from `input`.
    pub fn hash_reader(self, input: &mut dyn std::io::Read) -> std::io::Result<Vec<u8>> {
        match self {
            DigestHashFunction::Sha256 => {
                let mut hasher = sha2::Sha256::new();
                std::io::copy(input, &mut hasher)?;
                Ok(hasher.finalize().to_vec())
            }
        }
    }
}

impl fmt::Display for DigestHashFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
