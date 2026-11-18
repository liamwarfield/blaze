//! Port of `bazel/repository/downloader/Checksum` and `bazel/repository/cache/DownloadCache.KeyType`,
//! plus the `java.util.Base64` basic encoder/decoder they use.

use std::fmt;

use sha2::{Digest, Sha256};

/// The hash function of a [`Checksum`]. Port of `DownloadCache.KeyType`.
// Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:42 DownloadCache.KeyType
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyType {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
    Blake3,
}

impl KeyType {
    fn hex_len(self) -> usize {
        match self {
            KeyType::Sha1 => 40,
            KeyType::Sha256 | KeyType::Blake3 => 64,
            KeyType::Sha384 => 96,
            KeyType::Sha512 => 128,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:63 KeyType.isValid
    pub fn is_valid(self, checksum: Option<&str>) -> bool {
        match checksum {
            None => false,
            Some(c) => {
                !c.is_empty()
                    && c.len() == self.hex_len()
                    && c.bytes().all(|b| b.is_ascii_hexdigit())
            }
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:79 KeyType.getHashName
    pub fn get_hash_name(self) -> &'static str {
        match self {
            KeyType::Sha1 => "sha1",
            KeyType::Sha256 => "sha256",
            KeyType::Sha384 => "sha384",
            KeyType::Sha512 => "sha512",
            KeyType::Blake3 => "blake3",
        }
    }
}

impl fmt::Display for KeyType {
    // Java: com/google/devtools/build/lib/bazel/repository/cache/DownloadCache.java:83 KeyType.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            KeyType::Sha1 => "SHA-1",
            KeyType::Sha256 => "SHA-256",
            KeyType::Sha384 => "SHA-384",
            KeyType::Sha512 => "SHA-512",
            KeyType::Blake3 => "BLAKE3",
        })
    }
}

/// Exception thrown to indicate that a string is not a valid checksum for that key type.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/Checksum.java:30 Checksum.InvalidChecksumException
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidChecksumException(pub String);

impl fmt::Display for InvalidChecksumException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InvalidChecksumException {}

/// The content checksum for an HTTP download, which knows its own type. Port of `Checksum`.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/Checksum.java:28 Checksum
#[derive(Debug, Clone)]
pub struct Checksum {
    key_type: KeyType,
    /// The hash bytes (Guava `HashCode`).
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
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/Checksum.java:61 Checksum.fromString
    pub fn from_string(
        key_type: KeyType,
        hash: &str,
    ) -> Result<Checksum, InvalidChecksumException> {
        Self::from_string_sri(key_type, hash, false)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/Checksum.java:65 Checksum.fromString
    fn from_string_sri(
        key_type: KeyType,
        hash: &str,
        use_subresource_integrity: bool,
    ) -> Result<Checksum, InvalidChecksumException> {
        if !key_type.is_valid(Some(hash)) {
            return Err(InvalidChecksumException(format!(
                "Invalid {key_type} checksum '{hash}'"
            )));
        }
        let lower = hash.to_ascii_lowercase();
        Ok(Checksum {
            key_type,
            hash_code: hex::decode(lower).expect("validated hex"),
            use_subresource_integrity,
        })
    }

    /// The SHA-256 checksum of `bytes` (as `RegistryFileDownloadEvent.computeHash` does).
    pub fn sha256_of(bytes: &[u8]) -> Checksum {
        Checksum {
            key_type: KeyType::Sha256,
            hash_code: Sha256::digest(bytes).to_vec(),
            use_subresource_integrity: false,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/Checksum.java:75 Checksum.base64Decode
    fn base64_decode(data: &str) -> Result<Vec<u8>, InvalidChecksumException> {
        base64_decode(data)
            .map_err(|_| InvalidChecksumException(format!("Invalid base64 '{data}'")))
    }

    /// Constructs a new Checksum from a hash in Subresource Integrity format.
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/Checksum.java:84 Checksum.fromSubresourceIntegrity
    pub fn from_subresource_integrity(
        integrity: &str,
    ) -> Result<Checksum, InvalidChecksumException> {
        let (key_type, expected_length, hash) = if let Some(rest) = integrity.strip_prefix("sha1-")
        {
            (KeyType::Sha1, 20, Self::base64_decode(rest)?)
        } else if let Some(rest) = integrity.strip_prefix("sha256-") {
            (KeyType::Sha256, 32, Self::base64_decode(rest)?)
        } else if let Some(rest) = integrity.strip_prefix("sha384-") {
            (KeyType::Sha384, 48, Self::base64_decode(rest)?)
        } else if let Some(rest) = integrity.strip_prefix("sha512-") {
            (KeyType::Sha512, 64, Self::base64_decode(rest)?)
        } else if let Some(rest) = integrity.strip_prefix("blake3-") {
            (KeyType::Blake3, 32, Self::base64_decode(rest)?)
        } else {
            return Err(InvalidChecksumException(format!(
                "Unsupported checksum algorithm: '{integrity}' (expected SHA-1, SHA-256, SHA-384, or SHA-512)"
            )));
        };
        if hash.len() != expected_length {
            return Err(InvalidChecksumException(format!(
                "Invalid {key_type} SRI checksum '{integrity}'"
            )));
        }
        Self::from_string_sri(key_type, &hex::encode(hash), true)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/Checksum.java:121 Checksum.toSubresourceIntegrity
    pub fn to_subresource_integrity(&self) -> String {
        format!(
            "{}-{}",
            self.key_type.get_hash_name(),
            base64_encode(&self.hash_code)
        )
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
            format!(
                "{}-{}",
                self.key_type.get_hash_name(),
                base64_encode(other_hash)
            )
        } else {
            hex::encode(other_hash)
        }
    }
}

impl fmt::Display for Checksum {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/Checksum.java:132 Checksum.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(&self.hash_code))
    }
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `Base64.getEncoder().encodeToString` (RFC 4648, with padding).
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(BASE64_ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(BASE64_ALPHABET[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(BASE64_ALPHABET[(n >> 6) as usize & 63] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(BASE64_ALPHABET[n as usize & 63] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// `Base64.getDecoder().decode` (RFC 4648 basic; padding optional, no line separators). The error
/// carries Java's `IllegalArgumentException` message.
pub fn base64_decode(data: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u32> {
        BASE64_ALPHABET
            .iter()
            .position(|&a| a == c)
            .map(|p| p as u32)
    }
    let bytes = data.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let mut bits: u32 = 0;
    let mut nbits = 0;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'=' {
            // Padding: must be at a position where 2 or 3 chars of the last quantum were read.
            let rem = (i % 4) as i32;
            if rem < 2 || (rem == 2 && (i + 1 >= bytes.len() || bytes[i + 1] != b'=')) {
                return Err(
                    "Input byte array has an incorrect ending byte at ".to_string()
                        + &i.to_string(),
                );
            }
            let end = if rem == 2 { i + 2 } else { i + 1 };
            if end != bytes.len() {
                return Err(format!(
                    "Input byte array has incorrect ending byte at {}",
                    end
                ));
            }
            break;
        }
        let Some(v) = val(c) else {
            return Err(format!("Illegal base64 character {:x}", c));
        };
        bits = (bits << 6) | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((bits >> nbits) as u8);
            bits &= (1 << nbits) - 1;
        }
        i += 1;
    }
    if nbits == 6 {
        return Err("Last unit does not have enough valid bits".to_string());
    }
    Ok(out)
}

/// Hashes `content` with `key_type`'s hash function (`KeyType.newHasher`).
pub fn hash_with(key_type: KeyType, content: &[u8]) -> Vec<u8> {
    match key_type {
        KeyType::Sha256 => Sha256::digest(content).to_vec(),
        KeyType::Sha384 => sha2::Sha384::digest(content).to_vec(),
        KeyType::Sha512 => sha2::Sha512::digest(content).to_vec(),
        KeyType::Sha1 => blaze_util::unported!("Hashing.sha1"),
        KeyType::Blake3 => blaze_util::unported!("Blake3HashFunction"),
    }
}
