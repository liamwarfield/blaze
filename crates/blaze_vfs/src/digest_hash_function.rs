//! Port of `vfs/DigestHashFunction.java` with the functions a Bazel server registers: SHA-1 and
//! SHA-256 (`DigestHashFunction`), BLAKE3 and GITSHA1 (`vfs/bazel/BazelHashFunctions`).
//!
//! PORT: Java keeps an open registry (`register`) of Guava `HashFunction`s behind JCA
//! `MessageDigest` providers; the registry of a Bazel server is fixed at these four, so it is an
//! enum here, and each function's hasher is the matching Rust crate (`sha1`, `sha2`, `blake3`).

use std::fmt;
use std::io::Read;

use sha2::Digest;

/// A digest function for file contents (Java `DigestHashFunction`).
// Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:39 DigestHashFunction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DigestHashFunction {
    /// SHA-1 (`DigestHashFunction.SHA1`, names "SHA-1", "SHA1").
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:68 DigestHashFunction.SHA1
    Sha1,
    /// SHA-256 (`DigestHashFunction.SHA256`, names "SHA-256", "SHA256").
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:69 DigestHashFunction.SHA256
    Sha256,
    /// BLAKE3 (`BazelHashFunctions.BLAKE3`, name "BLAKE3").
    // Java: com/google/devtools/build/lib/vfs/bazel/BazelHashFunctions.java:25 BazelHashFunctions.BLAKE3
    Blake3,
    /// The SHA-1 of the Git blob of the content (`BazelHashFunctions.GITSHA1`, name "GITSHA1").
    // Java: com/google/devtools/build/lib/vfs/bazel/BazelHashFunctions.java:26 BazelHashFunctions.GITSHA1
    GitSha1,
}

// PORT: skipped, the registry is the enum (a server registers the same four functions): DigestHashFunction.register, DigestHashFunction.getPossibleHashFunctions
// PORT: skipped, JCA plumbing (each function's hasher is a Rust crate): DigestHashFunction.cloneOrCreateMessageDigest, DigestHashFunction.getMessageDigestInstance, DigestHashFunction.supportsClone
// PORT: skipped, JCA plumbing (each function's hasher is a Rust crate): BazelHashFunctions.ensureRegistered, Blake3Provider.<init>, GitSha1Provider.<init>
// PORT: skipped, the Guava HashFunction interface is DigestHashFunction::new_hasher and hash_bytes: GitSha1HashFunction.hashObject, GitSha1HashFunction.hashUnencodedChars
// PORT: skipped, the Guava HashFunction interface is DigestHashFunction::new_hasher and hash_bytes: GitSha1HashFunction.hashInt, GitSha1HashFunction.hashLong
// PORT: skipped, the Guava HashFunction interface is DigestHashFunction::new_hasher and hash_bytes: Blake3HashFunction.hashObject, Blake3HashFunction.hashUnencodedChars
// PORT: skipped, the Guava HashFunction interface is DigestHashFunction::new_hasher and hash_bytes: Blake3HashFunction.hashString, Blake3HashFunction.hashInt, Blake3HashFunction.hashLong

impl DigestHashFunction {
    /// The functions in registration order (`DigestHashFunction`'s statics, then
    /// `BazelHashFunctions`'s).
    pub const ALL: [DigestHashFunction; 4] = [
        DigestHashFunction::Sha1,
        DigestHashFunction::Sha256,
        DigestHashFunction::Blake3,
        DigestHashFunction::GitSha1,
    ];

    /// The registered names; the first is the name (`toString`).
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:169 DigestHashFunction.getNames
    pub fn names(self) -> &'static [&'static str] {
        match self {
            DigestHashFunction::Sha1 => &["SHA-1", "SHA1"],
            DigestHashFunction::Sha256 => &["SHA-256", "SHA256"],
            DigestHashFunction::Blake3 => &["BLAKE3"],
            DigestHashFunction::GitSha1 => &["GITSHA1"],
        }
    }

    /// The registered name ("SHA-256").
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:174 DigestHashFunction.toString
    pub fn name(self) -> &'static str {
        self.names()[0]
    }

    /// `DigestFunctionConverter.convert`: the function one of whose names equals `input`, ignoring
    /// case.
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:132 DigestHashFunction.DigestFunctionConverter.convert
    pub fn from_name(input: &str) -> Option<DigestHashFunction> {
        DigestHashFunction::ALL
            .into_iter()
            .find(|f| f.names().iter().any(|n| n.eq_ignore_ascii_case(input)))
    }

    /// `getDigestLength().getDigestMaximumLength()`: `bits() / 8`.
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:165 DigestHashFunction.getDigestLength
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:61 DigestHashFunction.DigestLength.DigestLengthImpl.getDigestMaximumLength
    // Java: com/google/devtools/build/lib/vfs/GitSha1HashFunction.java:46 GitSha1HashFunction.bits
    // Java: com/google/devtools/build/lib/vfs/bazel/Blake3HashFunction.java:30 Blake3HashFunction.bits
    pub fn digest_length(self) -> usize {
        match self {
            DigestHashFunction::Sha1 | DigestHashFunction::GitSha1 => 20,
            DigestHashFunction::Sha256 | DigestHashFunction::Blake3 => 32,
        }
    }

    /// `getHashFunction().newHasher()`.
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:148 DigestHashFunction.getHashFunction
    // Java: com/google/devtools/build/lib/vfs/GitSha1HashFunction.java:51 GitSha1HashFunction.newHasher
    // Java: com/google/devtools/build/lib/vfs/GitSha1HashFunction.java:56 GitSha1HashFunction.newHasher
    // Java: com/google/devtools/build/lib/vfs/bazel/Blake3HashFunction.java:35 Blake3HashFunction.newHasher
    // Java: com/google/devtools/build/lib/vfs/bazel/Blake3HashFunction.java:40 Blake3HashFunction.newHasher
    // Java: com/google/devtools/build/lib/vfs/bazel/Blake3Hasher.java:30 Blake3Hasher.<init>
    // Java: com/google/devtools/build/lib/vfs/GitSha1HashFunction.java:110 GitSha1HashFunction.DelayedGitSha1Hasher.<init>
    pub fn new_hasher(self) -> DigestHasher {
        DigestHasher(match self {
            DigestHashFunction::Sha1 => HasherState::Sha1(sha1::Sha1::new()),
            DigestHashFunction::Sha256 => HasherState::Sha256(sha2::Sha256::new()),
            DigestHashFunction::Blake3 => HasherState::Blake3(Box::new(blake3::Hasher::new())),
            DigestHashFunction::GitSha1 => HasherState::GitSha1(Vec::new()),
        })
    }

    /// `getHashFunction().hashBytes(input).asBytes()`.
    // Java: com/google/devtools/build/lib/vfs/GitSha1HashFunction.java:91 GitSha1HashFunction.hashBytes
    // Java: com/google/devtools/build/lib/vfs/GitSha1HashFunction.java:96 GitSha1HashFunction.hashBytes
    // Java: com/google/devtools/build/lib/vfs/GitSha1HashFunction.java:102 GitSha1HashFunction.hashBytes
    // Java: com/google/devtools/build/lib/vfs/bazel/Blake3HashFunction.java:75 Blake3HashFunction.hashBytes
    // Java: com/google/devtools/build/lib/vfs/bazel/Blake3HashFunction.java:80 Blake3HashFunction.hashBytes
    // Java: com/google/devtools/build/lib/vfs/bazel/Blake3HashFunction.java:86 Blake3HashFunction.hashBytes
    pub fn hash_bytes(self, input: &[u8]) -> Vec<u8> {
        if self == DigestHashFunction::GitSha1 {
            // `newHasher(len).putBytes(input).hash()`: the header with the length up front.
            return git_sha1(input);
        }
        let mut hasher = self.new_hasher();
        hasher.update(input);
        hasher.finish()
    }

    /// Hashes everything readable from `input` (`ByteSource.hash(getHashFunction())`, as
    /// `FileSystem.getDigest` does).
    pub fn hash_reader(self, input: &mut dyn Read) -> std::io::Result<Vec<u8>> {
        let mut hasher = self.new_hasher();
        let mut buffer = vec![0u8; 8192];
        loop {
            let n = match input.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            hasher.update(&buffer[..n]);
        }
        Ok(hasher.finish())
    }
}

/// A Guava `Hasher` of a [`DigestHashFunction`]: bytes in, digest out.
#[derive(Clone)]
pub struct DigestHasher(HasherState);

#[derive(Clone)]
enum HasherState {
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Blake3(Box<blake3::Hasher>),
    /// `GitSha1HashFunction.DelayedGitSha1Hasher`: the blob's bytes, hashed with their length
    /// in the header when the digest is asked for.
    GitSha1(Vec<u8>),
}

// PORT: skipped, Guava Hasher primitives (callers put bytes): GitSha1HashFunction.putBoolean, GitSha1HashFunction.putByte, GitSha1HashFunction.putShort
// PORT: skipped, Guava Hasher primitives (callers put bytes): GitSha1HashFunction.putInt, GitSha1HashFunction.putLong, GitSha1HashFunction.putFloat, GitSha1HashFunction.putDouble
// PORT: skipped, Guava Hasher primitives (callers put bytes): GitSha1HashFunction.putChar, GitSha1HashFunction.putUnencodedChars, GitSha1HashFunction.putString, GitSha1HashFunction.putObject
// PORT: skipped, Guava Hasher identity and ByteArrayOutput plumbing: GitSha1HashFunction.hashCode, GitSha1HashFunction.equals, GitSha1HashFunction.toByteArray
// PORT: skipped, a GitSha1 string is hashed from its bytes (no Rust caller hashes a Java string with a char-count header): GitSha1HashFunction.hashString
// PORT: skipped, Guava Hasher primitives (callers put bytes): Blake3Hasher.putByte, Blake3Hasher.putShort, Blake3Hasher.putInt, Blake3Hasher.putLong, Blake3Hasher.putChar
// PORT: skipped, Guava Hasher primitives (callers put bytes): Blake3Hasher.putFloat, Blake3Hasher.putDouble, Blake3Hasher.putBoolean, Blake3Hasher.putUnencodedChars
// PORT: skipped, Guava Hasher primitives (callers put bytes): Blake3Hasher.putString, Blake3Hasher.putObject
// PORT: skipped, JCA MessageDigest plumbing of the blake3 crate's hasher: Blake3MessageDigest.engineUpdate, Blake3MessageDigest.getOutput, Blake3MessageDigest.clone
// PORT: skipped, JCA MessageDigest plumbing of the blake3 crate's hasher: Blake3MessageDigest.engineReset, Blake3MessageDigest.engineGetDigestLength, Blake3MessageDigest.engineDigest
// PORT: skipped, a DigestLength interface with one implementation: DigestHashFunction.getDigestMaximumLength
// PORT: skipped, JNI of the C BLAKE3 library (the blake3 crate): Blake3MessageDigest.hasher_size, Blake3MessageDigest.initialize_hasher
// PORT: skipped, JNI of the C BLAKE3 library (the blake3 crate): Blake3MessageDigest.blake3_hasher_update, Blake3MessageDigest.blake3_hasher_finalize
// PORT: skipped, JCA MessageDigest plumbing (DigestHasher's GitSha1 state): GitSha1MessageDigest.engineUpdate, GitSha1MessageDigest.engineReset
// PORT: skipped, JCA MessageDigest plumbing (DigestHasher's GitSha1 state): GitSha1MessageDigest.engineDigest, GitSha1MessageDigest.internalReset

impl DigestHasher {
    /// `putBytes(bytes)`.
    // Java: com/google/devtools/build/lib/vfs/bazel/Blake3Hasher.java:36 Blake3Hasher.putBytes
    // Java: com/google/devtools/build/lib/vfs/bazel/Blake3Hasher.java:43 Blake3Hasher.putBytes
    // Java: com/google/devtools/build/lib/vfs/bazel/Blake3Hasher.java:50 Blake3Hasher.putBytes
    // Java: com/google/devtools/build/lib/vfs/GitSha1HashFunction.java:128 GitSha1HashFunction.DelayedGitSha1Hasher.putBytes
    // Java: com/google/devtools/build/lib/vfs/GitSha1HashFunction.java:135 GitSha1HashFunction.DelayedGitSha1Hasher.putBytes
    // Java: com/google/devtools/build/lib/vfs/GitSha1HashFunction.java:142 GitSha1HashFunction.DelayedGitSha1Hasher.putBytes
    pub fn update(&mut self, bytes: &[u8]) {
        match &mut self.0 {
            HasherState::Sha1(h) => h.update(bytes),
            HasherState::Sha256(h) => h.update(bytes),
            HasherState::Blake3(h) => {
                blake3::Hasher::update(h, bytes);
            }
            HasherState::GitSha1(buffer) => buffer.extend_from_slice(bytes),
        }
    }

    /// `hash().asBytes()`.
    // Java: com/google/devtools/build/lib/vfs/bazel/Blake3Hasher.java:64 Blake3Hasher.hash
    // Java: com/google/devtools/build/lib/vfs/GitSha1HashFunction.java:214 GitSha1HashFunction.DelayedGitSha1Hasher.hash
    pub fn finish(self) -> Vec<u8> {
        match self.0 {
            HasherState::Sha1(h) => h.finalize().to_vec(),
            HasherState::Sha256(h) => h.finalize().to_vec(),
            HasherState::Blake3(h) => blake3::Hasher::finalize(&h).as_bytes().to_vec(),
            HasherState::GitSha1(buffer) => git_sha1(&buffer),
        }
    }
}

impl std::io::Write for DigestHasher {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.update(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The SHA-1 of `"blob <size>\0"` followed by the content (`GitSha1HashFunction`'s
/// `newInitializedHasher(size)` and `DelayedGitSha1Hasher.hash`).
// Java: com/google/devtools/build/lib/vfs/GitSha1HashFunction.java:38 GitSha1HashFunction.newInitializedHasher
fn git_sha1(blob: &[u8]) -> Vec<u8> {
    let mut sha1 = sha1::Sha1::new();
    sha1.update(b"blob ");
    sha1.update(blob.len().to_string().as_bytes());
    sha1.update([0u8]);
    sha1.update(blob);
    sha1.finalize().to_vec()
}

impl fmt::Display for DigestHashFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The oracle's execution log of `cat pkg/in.txt > a.txt` with the input "hello\n" (6 bytes)
    /// under each `--digest_function`.
    #[test]
    fn digests_of_the_oracle_execution_log() {
        let input = b"hello\n";
        assert_eq!(
            hex(&DigestHashFunction::Blake3.hash_bytes(input)),
            "8e4c7c1b99dbfd50e7a95185fead5ee1448fa904a2fdd778eaf5f2dbfd629a99"
        );
        // `git hash-object` of the same content.
        assert_eq!(
            hex(&DigestHashFunction::GitSha1.hash_bytes(input)),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
        assert_eq!(
            hex(&DigestHashFunction::Sha1.hash_bytes(input)),
            "f572d396fae9206628714fb2ce00f72e94f2258f"
        );
        assert_eq!(
            hex(&DigestHashFunction::Sha256.hash_bytes(input)),
            "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"
        );
        for f in DigestHashFunction::ALL {
            // Streaming in pieces gives the one-shot digest.
            let mut hasher = f.new_hasher();
            hasher.update(b"hel");
            hasher.update(b"lo\n");
            assert_eq!(hasher.finish(), f.hash_bytes(input), "{f}");
            assert_eq!(f.hash_bytes(input).len(), f.digest_length(), "{f}");
            let mut reader: &[u8] = input;
            assert_eq!(f.hash_reader(&mut reader).unwrap(), f.hash_bytes(input));
        }
    }

    #[test]
    fn names_convert_ignoring_case() {
        assert_eq!(
            DigestHashFunction::from_name("sha-256"),
            Some(DigestHashFunction::Sha256)
        );
        assert_eq!(
            DigestHashFunction::from_name("Sha1"),
            Some(DigestHashFunction::Sha1)
        );
        assert_eq!(
            DigestHashFunction::from_name("blake3"),
            Some(DigestHashFunction::Blake3)
        );
        assert_eq!(
            DigestHashFunction::from_name("gitsha1"),
            Some(DigestHashFunction::GitSha1)
        );
        assert_eq!(DigestHashFunction::from_name("md5"), None);
        assert_eq!(DigestHashFunction::Sha1.to_string(), "SHA-1");
        assert_eq!(DigestHashFunction::GitSha1.to_string(), "GITSHA1");
    }
}
