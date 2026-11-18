//! Port of `Checksum`: the content checksum for an HTTP download, which knows its own type.

use std::fmt;

use base64::Engine as _;

use crate::cache::KeyType;

/// Exception thrown to indicate that a string is not a valid checksum for that key type.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/Checksum.java:31 Checksum.InvalidChecksumException
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidChecksumException {
    pub message: String,
}

impl fmt::Display for InvalidChecksumException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for InvalidChecksumException {}

fn invalid(message: String) -> InvalidChecksumException {
    InvalidChecksumException { message }
}

/// The content checksum for an HTTP download.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/Checksum.java:29 Checksum
#[derive(Clone, Debug)]
pub struct Checksum {
    key_type: KeyType,
    hash_code: Vec<u8>,
    use_subresource_integrity: bool,
}

impl PartialEq for Checksum {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/Checksum.java:139 Checksum.equals
    fn eq(&self, other: &Self) -> bool {
        self.key_type == other.key_type && self.hash_code == other.hash_code
    }
}

impl Eq for Checksum {}

impl std::hash::Hash for Checksum {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key_type.hash(state);
        self.hash_code.hash(state);
    }
}

impl Checksum {
    /// Constructs a new Checksum for a given key type and hash, in hex format.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/Checksum.java:63 Checksum.fromString
    pub fn from_string(
        key_type: KeyType,
        hash: &str,
    ) -> Result<Checksum, InvalidChecksumException> {
        Self::from_string_sri(key_type, hash, false)
    }

    fn from_string_sri(
        key_type: KeyType,
        hash: &str,
        use_subresource_integrity: bool,
    ) -> Result<Checksum, InvalidChecksumException> {
        if !key_type.is_valid(Some(hash)) {
            return Err(invalid(format!("Invalid {} checksum '{}'", key_type, hash)));
        }
        Ok(Checksum {
            key_type,
            hash_code: hex::decode(hash.to_ascii_lowercase()).expect("validated hex"),
            use_subresource_integrity,
        })
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/Checksum.java:77 Checksum.base64Decode
    fn base64_decode(data: &str) -> Result<Vec<u8>, InvalidChecksumException> {
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| invalid(format!("Invalid base64 '{}'", data)))
    }

    /// Constructs a new Checksum from a hash in Subresource Integrity format.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/Checksum.java:86 Checksum.fromSubresourceIntegrity
    pub fn from_subresource_integrity(
        integrity: &str,
    ) -> Result<Checksum, InvalidChecksumException> {
        let (key_type, expected_length, hash) = if let Some(r) = integrity.strip_prefix("sha1-") {
            (KeyType::SHA1, 20, Self::base64_decode(r)?)
        } else if let Some(r) = integrity.strip_prefix("sha256-") {
            (KeyType::SHA256, 32, Self::base64_decode(r)?)
        } else if let Some(r) = integrity.strip_prefix("sha384-") {
            (KeyType::SHA384, 48, Self::base64_decode(r)?)
        } else if let Some(r) = integrity.strip_prefix("sha512-") {
            (KeyType::SHA512, 64, Self::base64_decode(r)?)
        } else if let Some(r) = integrity.strip_prefix("blake3-") {
            (KeyType::BLAKE3, 32, Self::base64_decode(r)?)
        } else {
            return Err(invalid(format!(
                "Unsupported checksum algorithm: '{}' (expected SHA-1, SHA-256, SHA-384, or SHA-512)",
                integrity
            )));
        };
        if hash.len() != expected_length {
            return Err(invalid(format!(
                "Invalid {} SRI checksum '{}'",
                key_type, integrity
            )));
        }
        Self::from_string_sri(key_type, &hex::encode(hash), true)
    }

    fn to_sri(key_type: KeyType, hash_code: &[u8]) -> String {
        format!(
            "{}-{}",
            key_type.get_hash_name(),
            base64::engine::general_purpose::STANDARD.encode(hash_code)
        )
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/Checksum.java:127 Checksum.toSubresourceIntegrity
    pub fn to_subresource_integrity(&self) -> String {
        Self::to_sri(self.key_type, &self.hash_code)
    }

    pub fn get_hash_code(&self) -> &[u8] {
        &self.hash_code
    }

    pub fn get_key_type(&self) -> KeyType {
        self.key_type
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/Checksum.java:160 Checksum.emitOtherHashInSameFormat
    pub fn emit_other_hash_in_same_format(&self, other_hash: &[u8]) -> String {
        if self.use_subresource_integrity {
            Self::to_sri(self.key_type, other_hash)
        } else {
            hex::encode(other_hash)
        }
    }
}

impl fmt::Display for Checksum {
    /// The lowercase hex hash (`Checksum.toString()`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(&self.hash_code))
    }
}
