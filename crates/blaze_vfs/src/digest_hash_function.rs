//! Port of `DigestHashFunction` (SHA-256) and Bazel's `BazelHashFunctions.BLAKE3`.
//!
//! Java registers SHA-1 and SHA-256 in `DigestHashFunction`, and Bazel adds BLAKE3 and GITSHA1 in
//! `vfs/bazel/BazelHashFunctions`. SHA-256 and BLAKE3 are ported; SHA-1 and GITSHA1 are not (the
//! option converter fails loudly for their names).

use std::fmt;
use std::io::{self, Write};

use sha2::Digest;

/// A digest function for file contents (Java `DigestHashFunction`).
// Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:39 DigestHashFunction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DigestHashFunction {
    /// SHA-256.
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:69 DigestHashFunction.SHA256
    Sha256,
    /// BLAKE3 (256-bit output).
    ///
    /// PORT: Java registers BLAKE3 only when JNI is available (its hasher is native code); on
    /// Linux x86-64 it always is, so the Rust port always has it.
    // Java: com/google/devtools/build/lib/vfs/bazel/BazelHashFunctions.java:34 BazelHashFunctions.BLAKE3
    Blake3,
}

impl DigestHashFunction {
    /// Every ported, registered hash function.
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:197 DigestHashFunction.getPossibleHashFunctions
    pub const POSSIBLE_HASH_FUNCTIONS: [DigestHashFunction; 2] =
        [DigestHashFunction::Sha256, DigestHashFunction::Blake3];

    /// The registered canonical name ("SHA-256", "BLAKE3").
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:174 DigestHashFunction.toString
    pub fn name(self) -> &'static str {
        self.get_names()[0]
    }

    /// The canonical name followed by the alternative names the converter accepts.
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:169 DigestHashFunction.getNames
    pub fn get_names(self) -> &'static [&'static str] {
        match self {
            // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:69 DigestHashFunction.SHA256
            DigestHashFunction::Sha256 => &["SHA-256", "SHA256"],
            // Java: com/google/devtools/build/lib/vfs/bazel/BazelHashFunctions.java:34 BazelHashFunctions.<clinit>
            DigestHashFunction::Blake3 => &["BLAKE3"],
        }
    }

    /// The digest length in bytes (`getDigestLength().getDigestMaximumLength()`, which is
    /// `hashFunction.bits() / 8` for both ported functions).
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:58 DigestHashFunction.DigestLength.DigestLengthImpl.<init>
    pub fn get_digest_length(self) -> usize {
        match self {
            DigestHashFunction::Sha256 => 32,
            // Java: com/google/devtools/build/lib/vfs/bazel/Blake3HashFunction.java:31 Blake3HashFunction.bits
            DigestHashFunction::Blake3 => 256 / 8,
        }
    }

    /// A fresh hasher (`getHashFunction().newHasher()`).
    // Java: com/google/devtools/build/lib/vfs/bazel/Blake3HashFunction.java:36 Blake3HashFunction.newHasher
    pub fn new_hasher(self) -> DigestHasher {
        DigestHasher(match self {
            DigestHashFunction::Sha256 => HasherImpl::Sha256(sha2::Sha256::new()),
            DigestHashFunction::Blake3 => HasherImpl::Blake3(Box::new(blake3::Hasher::new())),
        })
    }

    /// `getHashFunction().hashBytes(input).asBytes()`.
    // Java: com/google/devtools/build/lib/vfs/bazel/Blake3HashFunction.java:76 Blake3HashFunction.hashBytes
    pub fn hash_bytes(self, input: &[u8]) -> Vec<u8> {
        let mut hasher = self.new_hasher();
        hasher.update(input);
        hasher.finish()
    }

    /// Hashes everything readable from `input`.
    pub fn hash_reader(self, input: &mut dyn io::Read) -> io::Result<Vec<u8>> {
        let mut hasher = self.new_hasher();
        io::copy(input, &mut hasher)?;
        Ok(hasher.finish())
    }

    /// Looks up a registered name, ignoring ASCII case (the registry lookup of
    /// `DigestFunctionConverter.convert`). Returns `None` for names not registered by a ported
    /// function.
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:133 DigestHashFunction.DigestFunctionConverter.convert
    pub fn for_name(input: &str) -> Option<DigestHashFunction> {
        DigestHashFunction::POSSIBLE_HASH_FUNCTIONS
            .into_iter()
            .find(|f| f.get_names().iter().any(|n| n.eq_ignore_ascii_case(input)))
    }
}

impl fmt::Display for DigestHashFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A streaming hasher for a [`DigestHashFunction`] (Guava `Hasher` / `MessageDigest`).
///
/// Writing to it feeds the hash; [`DigestHasher::finish`] returns the digest bytes.
#[derive(Clone)]
pub struct DigestHasher(HasherImpl);

#[derive(Clone)]
enum HasherImpl {
    Sha256(sha2::Sha256),
    // Boxed: blake3::Hasher is ~2 KB.
    Blake3(Box<blake3::Hasher>),
}

impl DigestHasher {
    /// Feeds `data` to the hash (`Hasher.putBytes`).
    // Java: com/google/devtools/build/lib/vfs/bazel/Blake3Hasher.java:52 Blake3Hasher.putBytes
    pub fn update(&mut self, data: &[u8]) {
        match &mut self.0 {
            HasherImpl::Sha256(h) => sha2::Digest::update(h, data),
            HasherImpl::Blake3(h) => {
                h.update(data);
            }
        }
    }

    /// The digest of everything fed so far (`Hasher.hash().asBytes()`).
    // Java: com/google/devtools/build/lib/vfs/bazel/Blake3Hasher.java:65 Blake3Hasher.hash
    pub fn finish(self) -> Vec<u8> {
        match self.0 {
            HasherImpl::Sha256(h) => sha2::Digest::finalize(h).to_vec(),
            HasherImpl::Blake3(h) => blake3::Hasher::finalize(&h).as_bytes().to_vec(),
        }
    }
}

impl Write for DigestHasher {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of `Blake3HasherTest` (`src/test/java/.../vfs/bazel/Blake3HasherTest.java`).
    mod blake3_hasher_test {
        use super::*;

        #[test]
        fn empty_hash() {
            let mut h = DigestHashFunction::Blake3.new_hasher();
            h.update(&[]);
            assert_eq!(
                hex::encode(h.finish()),
                "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
            );
        }

        #[test]
        fn hello_world() {
            let mut h = DigestHashFunction::Blake3.new_hasher();
            h.update(b"hello world");
            assert_eq!(
                hex::encode(h.finish()),
                "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24"
            );
        }
    }

    #[test]
    fn streaming_matches_one_shot() {
        for f in DigestHashFunction::POSSIBLE_HASH_FUNCTIONS {
            let mut h = f.new_hasher();
            h.update(b"hello ");
            h.update(b"world");
            assert_eq!(h.finish(), f.hash_bytes(b"hello world"));
        }
    }

    #[test]
    fn converter_names() {
        assert_eq!(
            DigestHashFunction::for_name("blake3"),
            Some(DigestHashFunction::Blake3)
        );
        assert_eq!(
            DigestHashFunction::for_name("sha256"),
            Some(DigestHashFunction::Sha256)
        );
        assert_eq!(
            DigestHashFunction::for_name("SHA-256"),
            Some(DigestHashFunction::Sha256)
        );
        assert_eq!(DigestHashFunction::for_name("md5"), None);
        assert_eq!(DigestHashFunction::Blake3.to_string(), "BLAKE3");
        assert_eq!(DigestHashFunction::Blake3.get_digest_length(), 32);
    }
}
