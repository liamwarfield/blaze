//! Port of `lib/remote/util/DigestUtil.java` and `DigestOutputStream.java`.

use std::cmp::Ordering;
use std::io::{self, Write};

use blaze_exec::actions::VirtualActionInput;
use blaze_protos::JavaMessage;
use blaze_vfs::{DigestHashFunction, FileStatus, Path};
use sha2::Digest as _;

use crate::common::remote_cache_client::ActionKey;
use crate::reapi::{Action, Digest, digest_function};

/// Utility methods to work with [`Digest`].
// Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:42 DigestUtil
#[derive(Debug, Clone)]
pub struct DigestUtil {
    // PORT: the XattrProvider is the file system's fast digest (`Path::get_digest`).
    hash_fn: DigestHashFunction,
    digest_function: digest_function::Value,
}

/// `DigestUtil.DIGEST_COMPARATOR`: by hash bytes (unsigned, lexicographic), then by size.
// Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:43 DigestUtil.DIGEST_COMPARATOR
pub fn digest_comparator(a: &Digest, b: &Digest) -> Ordering {
    a.hash
        .as_bytes()
        .cmp(b.hash.as_bytes())
        .then(a.size_bytes.cmp(&b.size_bytes))
}

impl DigestUtil {
    /// `new DigestUtil(xattrProvider, hashFn)`.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:51 DigestUtil.<init>
    pub fn new(hash_fn: DigestHashFunction) -> DigestUtil {
        DigestUtil {
            hash_fn,
            digest_function: get_digest_function_from_hash_function(hash_fn),
        }
    }

    /// Returns the currently used digest function.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:70 DigestUtil.getDigestFunction
    pub fn get_digest_function(&self) -> digest_function::Value {
        self.digest_function
    }

    /// The hash function (`hashFn`).
    pub fn get_hash_function(&self) -> DigestHashFunction {
        self.hash_fn
    }

    /// `compute(byte[] blob)`.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:74 DigestUtil.compute
    pub fn compute(&self, blob: &[u8]) -> Digest {
        DigestUtil::build_digest_from_bytes(&self.hash(blob), blob.len() as i64)
    }

    // PORT: `compute(byte[] data, int offset, int length)` (DigestUtil.java:89) is
    // `compute(&data[offset..offset + length])`, and `compute(ByteString blob)`
    // (DigestUtil.java:94) is `compute(&blob)`: a slice covers both.
    // PORT: the static `compute(DeterministicWriter input, HashFunction hashFunction)`
    // (DigestUtil.java:125) is folded into `compute_virtual_action_input` (the instance
    // overload, DigestUtil.java:136), its only caller; `hashCodeToString(HashCode)`
    // (DigestUtil.java:186) has no counterpart, the hashes are bytes here (`build_digest_from_bytes`
    // encodes them in lower-case hexadecimal).

    /// Computes a digest for a file.
    ///
    /// Prefer calling [`DigestUtil::compute_path_with_status`] when a recently obtained
    /// `FileStatus` is available.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:107 DigestUtil.compute
    pub fn compute_path(&self, path: &Path) -> io::Result<Digest> {
        let status = path.stat()?;
        self.compute_path_with_status(path, &status)
    }

    /// Computes a digest for a file, given a recently obtained file status.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:117 DigestUtil.compute
    pub fn compute_path_with_status(&self, path: &Path, status: &FileStatus) -> io::Result<Digest> {
        // Java: DigestUtils.getDigestWithManualFallback(path, xattrProvider, status)
        Ok(DigestUtil::build_digest_from_bytes(
            &path.get_digest()?,
            status.get_size(),
        ))
    }

    /// `compute(DeterministicWriter input)`: streams the input, as parameter files can be very
    /// large.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:122 DigestUtil.compute
    pub fn compute_virtual_action_input(
        &self,
        input: &dyn VirtualActionInput,
    ) -> io::Result<Digest> {
        let mut digest_output_stream = self.new_digest_output_stream(io::sink());
        input.write_to(&mut digest_output_stream)?;
        Ok(digest_output_stream.digest())
    }

    /// Computes a digest of the given proto message. Currently, we simply rely on message output
    /// as bytes, but this implementation relies on the stability of the proto encoding, in
    /// particular between different platforms and languages.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:142 DigestUtil.compute
    pub fn compute_message<M: JavaMessage>(&self, message: &M) -> Digest {
        self.compute(&message.to_byte_array())
    }

    /// `computeAsUtf8(str)`.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:146 DigestUtil.computeAsUtf8
    pub fn compute_as_utf8(&self, s: &str) -> Digest {
        self.compute(s.as_bytes())
    }

    /// `computeActionKey(action)`.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:150 DigestUtil.computeActionKey
    pub fn compute_action_key(&self, action: &Action) -> ActionKey {
        ActionKey::new(self.compute_message(action))
    }

    /// Assumes that the given Digest is a valid digest of an Action, and creates an ActionKey
    /// wrapper. This should not be called on the client side!
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:158 DigestUtil.asActionKey
    pub fn as_action_key(&self, digest: Digest) -> ActionKey {
        ActionKey::new(digest)
    }

    /// `asSpawnLogProto(actionKey)`.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:162 DigestUtil.asSpawnLogProto
    pub fn as_spawn_log_proto(
        &self,
        action_key: &ActionKey,
    ) -> blaze_protos::tools::protos::Digest {
        blaze_protos::tools::protos::Digest {
            hash: action_key.digest().hash.clone(),
            size_bytes: action_key.digest().size_bytes,
            hash_function_name: self.get_digest_function().as_str_name().to_string(),
        }
    }

    /// Returns the hash of `data` in binary.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:171 DigestUtil.hash
    pub fn hash(&self, data: &[u8]) -> Vec<u8> {
        match self.hash_fn {
            DigestHashFunction::Sha256 => sha2::Sha256::digest(data).to_vec(),
        }
    }

    /// `buildDigest(byte[] hash, long size)`.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:175 DigestUtil.buildDigest
    pub fn build_digest_from_bytes(hash: &[u8], size: i64) -> Digest {
        DigestUtil::build_digest(&hex::encode(hash), size)
    }

    /// `buildDigest(String hexHash, long size)`.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:179 DigestUtil.buildDigest
    pub fn build_digest(hex_hash: &str, size: i64) -> Digest {
        Digest {
            hash: hex_hash.to_string(),
            size_bytes: size,
        }
    }

    /// `newDigestOutputStream(out)`.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:187 DigestUtil.newDigestOutputStream
    pub fn new_digest_output_stream<W: Write>(&self, out: W) -> DigestOutputStream<W> {
        DigestOutputStream::new(self.hash_fn, out)
    }

    /// `DigestUtil.toString(digest)`: `hash/size`.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:191 DigestUtil.toString
    pub fn to_string(digest: &Digest) -> String {
        format!("{}/{}", digest.hash, digest.size_bytes)
    }

    /// `DigestUtil.fromString(digest)`.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:195 DigestUtil.fromString
    pub fn from_string(digest: &str) -> Digest {
        let parts: Vec<&str> = digest.split('/').collect();
        blaze_util::check_argument!(parts.len() == 2, "Invalid digest format: {}", digest);
        let size: i64 = parts[1].parse().unwrap_or_else(|_| {
            panic!("NumberFormatException: For input string: \"{}\"", parts[1])
        });
        DigestUtil::build_digest(parts[0], size)
    }

    /// `toBinaryDigest(digest)`.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:201 DigestUtil.toBinaryDigest
    pub fn to_binary_digest(digest: &Digest) -> Vec<u8> {
        hex::decode(&digest.hash).unwrap_or_else(|_| {
            panic!(
                "IllegalArgumentException: Illegal hexadecimal character in {}",
                digest.hash
            )
        })
    }

    /// `isOldStyleDigestFunction(digestFunction)`.
    // Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:205 DigestUtil.isOldStyleDigestFunction
    pub fn is_old_style_digest_function(digest_function: digest_function::Value) -> bool {
        // Old-style digest functions (SHA256, etc) are distinguishable by the length
        // of their hash alone and do not require extra specification, but newer
        // digest functions (which may have the same length hashes as the older
        // functions!) must be explicitly specified in the upload resource name.
        (digest_function as i32) <= 7
    }
}

// Java: com/google/devtools/build/lib/remote/util/DigestUtil.java:60 DigestUtil.getDigestFunctionFromHashFunction
fn get_digest_function_from_hash_function(hash_fn: DigestHashFunction) -> digest_function::Value {
    // Java: DigestHashFunction.SHA256 registers the names "SHA-256" and "SHA256".
    match hash_fn {
        DigestHashFunction::Sha256 => digest_function::Value::Sha256,
    }
}

/// An `OutputStream` that maintains a `Digest` of the data written to it.
// Java: com/google/devtools/build/lib/remote/util/DigestOutputStream.java:27 DigestOutputStream
pub struct DigestOutputStream<W: Write> {
    out: W,
    hasher: sha2::Sha256,
    size: i64,
}

impl<W: Write> DigestOutputStream<W> {
    /// Creates a new `DigestOutputStream`. The output stream should not be written to after
    /// [`DigestOutputStream::digest`] is called.
    // Java: com/google/devtools/build/lib/remote/util/DigestOutputStream.java:36 DigestOutputStream.<init>
    pub fn new(hash_function: DigestHashFunction, out: W) -> DigestOutputStream<W> {
        match hash_function {
            DigestHashFunction::Sha256 => DigestOutputStream {
                out,
                hasher: sha2::Sha256::new(),
                size: 0,
            },
        }
    }

    /// Returns the digest of the data written to this stream.
    // Java: com/google/devtools/build/lib/remote/util/DigestOutputStream.java:67 DigestOutputStream.digest
    pub fn digest(&self) -> Digest {
        DigestUtil::build_digest_from_bytes(&self.hasher.clone().finalize(), self.size)
    }

    /// The wrapped stream.
    pub fn get_mut(&mut self) -> &mut W {
        &mut self.out
    }

    /// Gives the wrapped stream back.
    pub fn into_inner(self) -> W {
        self.out
    }
}

impl<W: Write> Write for DigestOutputStream<W> {
    // Java: com/google/devtools/build/lib/remote/util/DigestOutputStream.java:49 DigestOutputStream.write
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // size += len; hasher.putBytes(bytes, off, len); out.write(bytes, off, len): the bytes
        // count and are hashed even when the write to the wrapped stream fails.
        self.size += buf.len() as i64;
        self.hasher.update(buf);
        self.out.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}
