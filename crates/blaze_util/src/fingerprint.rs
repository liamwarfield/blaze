//! Port of `com.google.devtools.build.lib.util.Fingerprint`.
//!
//! Java feeds a protobuf `CodedOutputStream` into a `MessageDigest`; the bytes hashed are the
//! protobuf "NoTag" encodings. This port writes the same bytes into the same digest.

use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256};

/// Digest functions Bazel supports for fingerprints (`vfs/DigestHashFunction`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigestHashFunction {
    Sha256,
    Sha1,
    Md5,
}

enum Hasher {
    Sha256(Sha256),
    Sha1(Sha1),
    Md5(Md5),
}

/// Java: com/google/devtools/build/lib/util/Fingerprint.java:39 Fingerprint
pub struct Fingerprint {
    hasher: Hasher,
}

impl Default for Fingerprint {
    fn default() -> Self {
        Self::new()
    }
}

impl Fingerprint {
    /// Java: Fingerprint.java Fingerprint() — SHA-256.
    pub fn new() -> Self {
        Self::with_digest_function(DigestHashFunction::Sha256)
    }

    pub fn with_digest_function(f: DigestHashFunction) -> Self {
        let hasher = match f {
            DigestHashFunction::Sha256 => Hasher::Sha256(Sha256::new()),
            DigestHashFunction::Sha1 => Hasher::Sha1(Sha1::new()),
            DigestHashFunction::Md5 => Hasher::Md5(Md5::new()),
        };
        Fingerprint { hasher }
    }

    fn write(&mut self, bytes: &[u8]) {
        match &mut self.hasher {
            Hasher::Sha256(h) => h.update(bytes),
            Hasher::Sha1(h) => h.update(bytes),
            Hasher::Md5(h) => h.update(bytes),
        }
    }

    fn write_varint64(&mut self, mut v: u64) {
        let mut buf = [0u8; 10];
        let mut i = 0;
        loop {
            if v < 0x80 {
                buf[i] = v as u8;
                i += 1;
                break;
            }
            buf[i] = ((v & 0x7f) as u8) | 0x80;
            v >>= 7;
            i += 1;
        }
        self.write(&buf[..i]);
    }

    /// Java: Fingerprint.digestAndReset
    pub fn digest_and_reset(&mut self) -> Vec<u8> {
        match &mut self.hasher {
            Hasher::Sha256(h) => h.finalize_reset().to_vec(),
            Hasher::Sha1(h) => h.finalize_reset().to_vec(),
            Hasher::Md5(h) => h.finalize_reset().to_vec(),
        }
    }

    /// Java: Fingerprint.hexDigestAndReset
    pub fn hex_digest_and_reset(&mut self) -> String {
        hex::encode(self.digest_and_reset())
    }

    /// Java: Fingerprint.addBytes(byte[]) — raw bytes, no length prefix.
    pub fn add_bytes(&mut self, input: &[u8]) -> &mut Self {
        self.write(input);
        self
    }

    /// Java: Fingerprint.addBoolean (CodedOutputStream.writeBoolNoTag)
    pub fn add_boolean(&mut self, input: bool) -> &mut Self {
        self.write(&[input as u8]);
        self
    }

    /// Java: Fingerprint.addNullableBoolean
    pub fn add_nullable_boolean(&mut self, input: Option<bool>) -> &mut Self {
        match input {
            None => self.add_boolean(false),
            Some(b) => self.add_boolean(true).add_boolean(b),
        }
    }

    /// Java: Fingerprint.addInt (writeInt32NoTag: negative values are sign-extended to 10 bytes)
    pub fn add_int(&mut self, x: i32) -> &mut Self {
        if x >= 0 {
            self.write_varint64(x as u64);
        } else {
            self.write_varint64(x as i64 as u64);
        }
        self
    }

    /// Java: Fingerprint.addLong (writeInt64NoTag)
    pub fn add_long(&mut self, x: i64) -> &mut Self {
        self.write_varint64(x as u64);
        self
    }

    /// Java: Fingerprint.addNullableInt
    pub fn add_nullable_int(&mut self, input: Option<i32>) -> &mut Self {
        match input {
            None => self.add_boolean(false),
            Some(i) => self.add_boolean(true).add_int(i),
        }
    }

    /// Java: Fingerprint.addString (writeStringNoTag: varint UTF-8 length, then UTF-8 bytes).
    ///
    /// Invariant (decisions.md 2026-11-18 "Strings"): strings reaching here are internal-form, as
    /// the Java strings at the same call are (one char `<= U+00FF` per byte,
    /// `blaze_util::string_encoding`). The UTF-8 of such a string is exactly what Java hashes:
    /// `writeStringNoTag` UTF-8-encodes the Latin-1 chars, so each byte `>= 0x80` of the original
    /// text takes two bytes (the "double encoding" seen in the oracle's action keys).
    ///
    /// Until the decoding switch the caller's string is still Unicode;
    /// `string_encoding::to_internal` (the migration seam) bridges that, here and nowhere else.
    /// This is the one string method: there is no second encoding to choose at a call site.
    pub fn add_string(&mut self, input: &str) -> &mut Self {
        let input = crate::string_encoding::to_internal(input);
        self.write_varint64(input.len() as u64);
        self.write(input.as_bytes());
        self
    }

    /// The same as [`Fingerprint::add_string`].
    // PORT: transitional. Before the string model was decided, `add_string` hashed the Rust
    // string's bytes and this method re-created Java's double encoding by hand; call sites chose
    // between the two (review 2026-11-18-actions S1-1). Both now do what Java's `addString` does.
    // blaze_util, blaze_collect, blaze_actions and blaze_cc_features call only `add_string`; this
    // name stays for the other crates' callers and goes away with the last of them.
    pub fn add_internal_string(&mut self, input: &str) -> &mut Self {
        self.add_string(input)
    }

    /// Java: Fingerprint.addNullableString
    pub fn add_nullable_string(&mut self, input: Option<&str>) -> &mut Self {
        match input {
            None => self.add_boolean(false),
            Some(s) => self.add_boolean(true).add_string(s),
        }
    }

    /// Java: Fingerprint.addStrings
    pub fn add_strings<I, S>(&mut self, inputs: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        I::IntoIter: ExactSizeIterator,
        S: AsRef<str>,
    {
        let it = inputs.into_iter();
        self.add_int(it.len() as i32);
        for s in it {
            self.add_string(s.as_ref());
        }
        self
    }

    /// Java: Fingerprint.addIterableStrings
    pub fn add_iterable_strings<I, S>(&mut self, inputs: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for s in inputs {
            self.add_boolean(true);
            self.add_string(s.as_ref());
        }
        self.add_boolean(false)
    }

    /// Java: Fingerprint.addStringMap (iteration order of the given map)
    pub fn add_string_map<'a, I>(&mut self, inputs: I) -> &mut Self
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
        I::IntoIter: ExactSizeIterator,
    {
        let it = inputs.into_iter();
        self.add_int(it.len() as i32);
        for (k, v) in it {
            self.add_string(k);
            self.add_string(v);
        }
        self
    }

    /// Java: Fingerprint.addUUID
    pub fn add_uuid(&mut self, least_significant: i64, most_significant: i64) -> &mut Self {
        self.add_long(least_significant).add_long(most_significant)
    }

    /// Java: Fingerprint.getHexDigest — SHA-256 of `input.getBytes(UTF_8)`, hex. The same
    /// invariant and seam as [`Fingerprint::add_string`].
    pub fn get_hex_digest(input: &str) -> String {
        let input = crate::string_encoding::to_internal(input);
        hex::encode(Sha256::digest(input.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected digests computed with protobuf CodedOutputStream semantics.
    #[test]
    fn varint_encodings() {
        let mut a = Fingerprint::new();
        a.add_int(-1);
        let mut b = Fingerprint::new();
        b.add_bytes(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]);
        assert_eq!(a.digest_and_reset(), b.digest_and_reset());

        let mut a = Fingerprint::new();
        a.add_int(300).add_string("hi").add_boolean(true);
        let mut b = Fingerprint::new();
        b.add_bytes(&[0xac, 0x02, 0x02, b'h', b'i', 0x01]);
        assert_eq!(a.digest_and_reset(), b.digest_and_reset());
    }

    #[test]
    fn hex_digest_of_empty() {
        assert_eq!(
            Fingerprint::new().hex_digest_and_reset(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    const E_ACUTE_TWICE: [u8; 5] = [4, 0xc3, 0x83, 0xc2, 0xa9];

    fn all_string_methods(s: &str) -> Vec<u8> {
        let mut a = Fingerprint::new();
        a.add_string(s);
        a.add_internal_string(s);
        a.add_strings([s]);
        a.add_string_map([(s, s)]);
        a.add_iterable_strings([s]);
        a.add_nullable_string(Some(s));
        a.digest_and_reset()
    }

    fn all_string_methods_expected() -> Vec<u8> {
        let e = E_ACUTE_TWICE;
        let mut b = Fingerprint::new();
        b.add_bytes(&e);
        b.add_bytes(&e);
        b.add_bytes(&[1]).add_bytes(&e);
        b.add_bytes(&[1]).add_bytes(&e).add_bytes(&e);
        b.add_bytes(&[1]).add_bytes(&e).add_bytes(&[0]);
        b.add_bytes(&[1]).add_bytes(&e);
        b.digest_and_reset()
    }

    /// Java: `new Fingerprint().addString(unicodeToInternal("é"))` hashes the varint 4 and the
    /// bytes c3 83 c2 a9 (`writeStringNoTag` of the Latin-1 chars c3, a9). After the decoding
    /// switch the caller passes that internal string.
    #[test]
    fn add_string_of_internal_string_is_javas_double_encoding() {
        use crate::string_encoding::{unicode_to_internal, with_strings_are_internal_for_testing};
        let internal = unicode_to_internal("é");
        assert_eq!(
            internal.chars().map(|c| c as u32).collect::<Vec<_>>(),
            [0xc3, 0xa9]
        );
        with_strings_are_internal_for_testing(true, || {
            assert_eq!(all_string_methods(&internal), all_string_methods_expected());
            assert_eq!(
                Fingerprint::get_hex_digest(&internal),
                hex::encode(Sha256::digest([0xc3, 0x83, 0xc2, 0xa9]))
            );
        });
    }

    /// Before the switch the caller still holds the Unicode string; the seam gives the same
    /// bytes.
    #[test]
    fn add_string_of_unicode_string_before_the_switch() {
        use crate::string_encoding::with_strings_are_internal_for_testing;
        with_strings_are_internal_for_testing(false, || {
            assert_eq!(all_string_methods("é"), all_string_methods_expected());
            assert_eq!(
                Fingerprint::get_hex_digest("é"),
                hex::encode(Sha256::digest([0xc3, 0x83, 0xc2, 0xa9]))
            );
        });
    }
}
