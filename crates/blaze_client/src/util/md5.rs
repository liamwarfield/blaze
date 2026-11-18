//! Port of `src/main/cpp/util/md5.h` (`blaze_util::Md5Digest`), on top of the `md-5` crate.

use md5::{Digest, Md5};

// Java: src/main/cpp/util/md5.h:39 Md5Digest
/// Incremental MD5 computation.
#[derive(Clone)]
pub struct Md5Digest {
    hasher: Md5,
    digest: Option<[u8; 16]>,
}

impl Md5Digest {
    /// The MD5 digest is always 128 bits = 16 bytes.
    pub const K_DIGEST_LENGTH: usize = 16;

    pub fn new() -> Self {
        Md5Digest {
            hasher: Md5::new(),
            digest: None,
        }
    }

    // Java: src/main/cpp/util/md5.h:51 Md5Digest::Reset
    /// Resets the context so that it can be used to calculate another MD5 digest.
    pub fn reset(&mut self) {
        self.hasher = Md5::new();
        self.digest = None;
    }

    // Java: src/main/cpp/util/md5.h:55 Md5Digest::Update
    /// Adds `buf` to the MD5 digest.
    pub fn update(&mut self, buf: &[u8]) {
        self.hasher.update(buf);
    }

    // Java: src/main/cpp/util/md5.h:58 Md5Digest::Finish
    /// Retrieves the computed MD5 digest as a 16 byte array.
    pub fn finish(&mut self) -> [u8; 16] {
        let result = self.hasher.finalize_reset();
        let mut out = [0u8; 16];
        out.copy_from_slice(&result);
        self.digest = Some(out);
        out
    }

    // Java: src/main/cpp/util/md5.cc:342 Md5Digest::String
    /// Produces a hexadecimal string representation of this digest in the form `[0-9a-f]{32}`.
    pub fn string(&self) -> String {
        // PORT: the C++ prints the internal state, which equals the digest after Finish(). The
        // state before Finish() is not exposed by the md-5 crate.
        match &self.digest {
            Some(d) => hex::encode(d),
            None => blaze_util::unported!("Md5Digest::String", "called before Finish()"),
        }
    }
}
