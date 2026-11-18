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
        java_base64_decode(data).map_err(|_| invalid(format!("Invalid base64 '{}'", data)))
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

/// `java.util.Base64.getDecoder().decode(String)`: the RFC 4648 "basic" decoder. Unlike a strict
/// decoder it accepts input without padding (`xx`, `xxx` as the last unit) and does not check
/// that the unused bits of the last unit are zero; it rejects every byte outside the alphabet
/// (no line separators), a dangling single character, incomplete padding (`xx=`), padding at
/// the start of a unit, and anything after the padding. The error carries Java's
/// `IllegalArgumentException` message.
// Java: java.base/java/util/Base64.java Base64.Decoder.decode0 (the unrolled fast path
// decodes the same language and is not ported; expected values checked against OpenJDK 25)
fn java_base64_decode(data: &str) -> Result<Vec<u8>, String> {
    /// `Base64.Decoder.fromBase64`: the value of an alphabet byte, -2 for `=`, else -1.
    fn from_base64(b: u8) -> i32 {
        match b {
            b'A'..=b'Z' => (b - b'A') as i32,
            b'a'..=b'z' => (b - b'a') as i32 + 26,
            b'0'..=b'9' => (b - b'0') as i32 + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => -2,
            _ => -1,
        }
    }

    // src.getBytes(ISO_8859_1): a char outside Latin-1 becomes '?'.
    let src: Vec<u8> = data
        .chars()
        .map(|c| {
            if (c as u32) < 0x100 {
                c as u32 as u8
            } else {
                b'?'
            }
        })
        .collect();
    let sl = src.len();
    // Base64.Decoder.decodedOutLength
    if sl == 0 {
        return Ok(Vec::new());
    }
    if sl < 2 {
        return Err("Input byte[] should at least have 2 bytes for base64 bytes".to_string());
    }

    let mut dst: Vec<u8> = Vec::with_capacity(3 * sl.div_ceil(4));
    let mut sp = 0;
    let mut bits: i32 = 0;
    let mut shiftto: i32 = 18; // pos of first byte of 4-byte atom
    while sp < sl {
        let b = from_base64(src[sp]);
        sp += 1;
        if b < 0 {
            if b == -2 {
                // padding byte '='
                // =     shiftto==18 unnecessary padding
                // x=    shiftto==12 a dangling single x
                // x     to be handled together with non-padding case
                // xx=   shiftto==6&&sp==sl missing last =
                // xx=y  shiftto==6 last is not =
                if shiftto == 6
                    && (sp == sl || {
                        let next = src[sp];
                        sp += 1;
                        next != b'='
                    })
                    || shiftto == 18
                {
                    return Err("Input byte array has wrong 4-byte ending unit".to_string());
                }
                break;
            }
            // Integer.toString(src[sp - 1], 16) of a (signed) byte.
            let illegal = src[sp - 1] as i8 as i32;
            return Err(format!(
                "Illegal base64 character {}{:x}",
                if illegal < 0 { "-" } else { "" },
                illegal.unsigned_abs()
            ));
        }
        bits |= b << shiftto;
        shiftto -= 6;
        if shiftto < 0 {
            dst.push((bits >> 16) as u8);
            dst.push((bits >> 8) as u8);
            dst.push(bits as u8);
            shiftto = 18;
            bits = 0;
        }
    }
    // reached end of byte array or hit padding '=' characters.
    if shiftto == 6 {
        dst.push((bits >> 16) as u8);
    } else if shiftto == 0 {
        dst.push((bits >> 16) as u8);
        dst.push((bits >> 8) as u8);
    } else if shiftto == 12 {
        // dangling single "x", incorrectly encoded.
        return Err("Last unit does not have enough valid bits".to_string());
    }
    // anything left is invalid, if is not MIME.
    if sp < sl {
        return Err(format!(
            "Input byte array has incorrect ending byte at {}",
            sp
        ));
    }
    Ok(dst)
}

#[cfg(test)]
mod tests {
    //! Bazel has no `ChecksumTest`; the expected values follow `Checksum.java:75-119` and
    //! `java.util.Base64.Decoder` (basic alphabet).
    use super::*;

    const HELLO_SHA256_HEX: &str =
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
    const HELLO_SHA256_SRI: &str = "sha256-LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ=";
    const HELLO_SHA512_SRI: &str = "sha512-m3HSJL1i83hdltRq0+o9czGb+8KJDKra4t/3JRlnPKcjI8PZm6XBHXx6zG4UuMXaDEZjR1wuXDre9G9zvN7AQw==";

    fn invalid_base64(integrity: &str) {
        let data = integrity.split_once('-').unwrap().1;
        assert_eq!(
            Checksum::from_subresource_integrity(integrity)
                .unwrap_err()
                .message,
            format!("Invalid base64 '{}'", data),
        );
    }

    #[test]
    fn from_subresource_integrity_padded() {
        let c = Checksum::from_subresource_integrity(HELLO_SHA256_SRI).unwrap();
        assert_eq!(c.get_key_type(), KeyType::SHA256);
        assert_eq!(c.to_string(), HELLO_SHA256_HEX);
        assert_eq!(c.to_subresource_integrity(), HELLO_SHA256_SRI);
    }

    #[test]
    fn from_subresource_integrity_accepts_missing_padding() {
        // 43 characters: the padded form without its '='.
        let unpadded = HELLO_SHA256_SRI.strip_suffix('=').unwrap();
        assert_eq!(unpadded.len(), "sha256-".len() + 43);
        let c = Checksum::from_subresource_integrity(unpadded).unwrap();
        assert_eq!(
            c,
            Checksum::from_subresource_integrity(HELLO_SHA256_SRI).unwrap()
        );
        assert_eq!(c.to_string(), HELLO_SHA256_HEX);
        // The canonical form is printed again.
        assert_eq!(c.to_subresource_integrity(), HELLO_SHA256_SRI);

        // 86 characters + "==".
        let padded = Checksum::from_subresource_integrity(HELLO_SHA512_SRI).unwrap();
        let unpadded = HELLO_SHA512_SRI.strip_suffix("==").unwrap();
        assert_eq!(
            Checksum::from_subresource_integrity(unpadded).unwrap(),
            padded
        );
        assert_eq!(padded.get_key_type(), KeyType::SHA512);
    }

    #[test]
    fn from_subresource_integrity_does_not_check_unused_bits() {
        // java.util.Base64 ignores the two unused bits of the last character ('Q' -> 'R').
        let c = Checksum::from_subresource_integrity(
            "sha256-LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCR=",
        )
        .unwrap();
        assert_eq!(c.to_string(), HELLO_SHA256_HEX);
    }

    #[test]
    fn from_subresource_integrity_rejects_malformed_base64() {
        // "xx=": missing last '='.
        invalid_base64(HELLO_SHA512_SRI.strip_suffix('=').unwrap());
        // "xxx=y": something after the padding.
        invalid_base64("sha256-LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ=A");
        invalid_base64("sha256-LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ==");
        // "=": padding at the start of a unit.
        invalid_base64("sha256-LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQA=");
        // A dangling single character.
        invalid_base64("sha256-LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQAB");
        invalid_base64("sha256-L");
        // Bytes outside the basic alphabet: URL-safe characters, white space, line separators.
        invalid_base64("sha256-LPJNul-wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ=");
        invalid_base64("sha256-LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ= ");
        invalid_base64("sha256-LPJNul+wow4m6Dsqxbninh\nsWHlwfp0JecwQzYpOLmCQ=");
        invalid_base64("sha256- LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ=");
    }

    #[test]
    fn from_subresource_integrity_wrong_length() {
        assert_eq!(
            Checksum::from_subresource_integrity("sha256-")
                .unwrap_err()
                .message,
            "Invalid SHA-256 SRI checksum 'sha256-'"
        );
        assert_eq!(
            Checksum::from_subresource_integrity(
                "sha1-LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ"
            )
            .unwrap_err()
            .message,
            "Invalid SHA-1 SRI checksum 'sha1-LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ'"
        );
    }

    #[test]
    fn from_subresource_integrity_unsupported_algorithm() {
        assert_eq!(
            Checksum::from_subresource_integrity("md5-XUFAKrxLKna5cZ2REBfFkg==")
                .unwrap_err()
                .message,
            "Unsupported checksum algorithm: 'md5-XUFAKrxLKna5cZ2REBfFkg==' (expected SHA-1, SHA-256, SHA-384, or SHA-512)"
        );
    }

    #[test]
    fn java_base64_decode_matches_java_util_base64() {
        assert_eq!(java_base64_decode(""), Ok(vec![]));
        assert_eq!(java_base64_decode("aGVsbG8="), Ok(b"hello".to_vec()));
        assert_eq!(java_base64_decode("aGVsbG8"), Ok(b"hello".to_vec()));
        assert_eq!(java_base64_decode("aGk="), Ok(b"hi".to_vec()));
        assert_eq!(java_base64_decode("aGk"), Ok(b"hi".to_vec()));
        assert_eq!(java_base64_decode("aA=="), Ok(b"h".to_vec()));
        assert_eq!(java_base64_decode("aA"), Ok(b"h".to_vec()));
        assert_eq!(
            java_base64_decode("a"),
            Err("Input byte[] should at least have 2 bytes for base64 bytes".to_string())
        );
        assert_eq!(
            java_base64_decode("aGVsb"),
            Err("Last unit does not have enough valid bits".to_string())
        );
        assert_eq!(
            java_base64_decode("aA="),
            Err("Input byte array has wrong 4-byte ending unit".to_string())
        );
        assert_eq!(
            java_base64_decode("aA=x"),
            Err("Input byte array has wrong 4-byte ending unit".to_string())
        );
        assert_eq!(
            java_base64_decode("aGk=aGk="),
            Err("Input byte array has incorrect ending byte at 4".to_string())
        );
        assert_eq!(
            java_base64_decode("aGVs=G8="),
            Err("Input byte array has wrong 4-byte ending unit".to_string())
        );
        assert_eq!(
            java_base64_decode("aG_s"),
            Err("Illegal base64 character 5f".to_string())
        );
        assert_eq!(
            java_base64_decode("aG\u{e9}s"),
            Err("Illegal base64 character -17".to_string())
        );
    }
}
