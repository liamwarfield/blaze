//! Port of `src/test/java/com/google/devtools/build/lib/util/StringEncodingTest.java`, plus
//! vectors for the JDK decoder that `internalToUnicode` relies on.
//!
//! Java's `new String(s.getBytes(UTF_8), ISO_8859_1)` is `latin1_decode(s.as_bytes())` here, and
//! `isSameInstanceAs` is `Cow::Borrowed` of the same slice.
//!
//! PORT: platform strings. Blaze always has the behaviour of the test's
//! `SUN_JNU_ENCODING.equals(ISO_8859_1) && OS.getCurrent() == OS.LINUX` branch (see the module
//! comment of `string_encoding`), so the `else` branches of the two platform tests are not ported.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use blaze_util::string_encoding::{
    internal_to_platform, internal_to_unicode, is_internal, java_new_string_utf8, latin1_decode,
    latin1_encode, platform_to_internal, unicode_to_internal,
};

/// `new String(s.getBytes(UTF_8), ISO_8859_1)`
fn utf8_as_latin1(s: &str) -> String {
    latin1_decode(s.as_bytes()).into_owned()
}

// The point is whether the `Cow` borrows, so it is taken as it is.
#[allow(clippy::ptr_arg)]
fn is_same_instance(got: &Cow<'_, str>, want: &str) -> bool {
    matches!(got, Cow::Borrowed(b) if std::ptr::eq(*b, want))
}

#[allow(clippy::ptr_arg)]
fn is_same_platform_instance(got: &Cow<'_, OsStr>, want: &[u8]) -> bool {
    matches!(got, Cow::Borrowed(b) if std::ptr::eq(b.as_bytes(), want))
}

#[test]
fn test_unicode_to_internal() {
    let empty = "";
    assert!(is_same_instance(&unicode_to_internal(empty), empty));
    let hello = "hello";
    assert!(is_same_instance(&unicode_to_internal(hello), hello));
    assert_eq!(unicode_to_internal("hällo"), utf8_as_latin1("hällo"));
    assert_eq!(unicode_to_internal("hållo"), utf8_as_latin1("hållo"));
    assert_eq!(unicode_to_internal("h👋llo"), utf8_as_latin1("h👋llo"));
}

#[test]
fn test_internal_to_unicode() {
    let empty = "";
    assert!(is_same_instance(&internal_to_unicode(empty), empty));
    let hello = "hello";
    assert!(is_same_instance(&internal_to_unicode(hello), hello));
    assert_eq!(internal_to_unicode(&utf8_as_latin1("hällo")), "hällo");
    assert_eq!(internal_to_unicode(&utf8_as_latin1("hållo")), "hållo");
    assert_eq!(internal_to_unicode(&utf8_as_latin1("h👋llo")), "h👋llo");
}

#[test]
fn test_platform_to_internal() {
    // Java checks that the platform string is returned as the same instance; in Rust the platform
    // string is the byte string and the internal string has one char per byte.
    let empty = "";
    assert!(is_same_instance(
        &platform_to_internal(OsStr::new(empty)),
        empty
    ));
    let hello = "hello";
    assert!(is_same_instance(
        &platform_to_internal(OsStr::new(hello)),
        hello
    ));
    for s in ["hällo", "hållo", "h👋llo"] {
        assert_eq!(
            platform_to_internal(OsStr::from_bytes(s.as_bytes())),
            utf8_as_latin1(s)
        );
    }
    {
        // Not valid Unicode.
        let bytes = [0xFF, 0xFE, 0x01];
        let s = platform_to_internal(OsStr::from_bytes(&bytes));
        assert_eq!(s, "\u{ff}\u{fe}\u{1}");
    }
}

#[test]
fn test_internal_to_platform() {
    let empty = "";
    assert!(is_same_platform_instance(
        &internal_to_platform(empty),
        empty.as_bytes()
    ));
    let hello = "hello";
    assert!(is_same_platform_instance(
        &internal_to_platform(hello),
        hello.as_bytes()
    ));
    for s in ["hällo", "hållo", "h👋llo"] {
        assert_eq!(
            internal_to_platform(&utf8_as_latin1(s)).as_bytes(),
            s.as_bytes()
        );
    }
    {
        // Not valid Unicode.
        let s = latin1_decode(&[0xFF, 0xFE, 0x01]);
        assert_eq!(internal_to_platform(&s).as_bytes(), [0xFF, 0xFE, 0x01]);
    }
}

/// Java: `testPlatformToInternal_roundtrip`, `@TestParameter({"ascii", "äöüÄÖÜß", "🌱",
/// "羅勒罗勒学名"})`. The Java test assumes the string can be encoded in `sun.jnu.encoding`; with
/// Latin-1 that leaves the first two (as Latin-1 bytes). The other two are run as the UTF-8 bytes
/// a file name would have.
#[test]
fn test_platform_to_internal_roundtrip() {
    let latin1_platform: Vec<Vec<u8>> = ["ascii", "äöüÄÖÜß"]
        .iter()
        .map(|s| s.chars().map(|c| c as u8).collect())
        .collect();
    let utf8_platform: Vec<Vec<u8>> = ["🌱", "羅勒罗勒学名"]
        .iter()
        .map(|s| s.as_bytes().to_vec())
        .collect();
    for platform in latin1_platform.iter().chain(&utf8_platform) {
        let internal = platform_to_internal(OsStr::from_bytes(platform));
        // In the internal encoding, raw bytes are encoded as Latin-1.
        assert!(is_internal(&internal));
        let roundtripped = internal_to_platform(&internal);
        assert_eq!(roundtripped.as_bytes(), &platform[..]);
        if platform.is_ascii() {
            assert!(matches!(internal, Cow::Borrowed(_)));
            assert!(matches!(roundtripped, Cow::Borrowed(_)));
        }
    }
}

#[test]
fn test_platform_to_internal_raw_bytes_roundtrip() {
    // Not valid UTF-8
    let raw_bytes = [0x00, 0x7F, 0x80, 0xFE, 0xFF];
    assert!(String::from_utf8(raw_bytes.to_vec()).is_err());

    let internal = platform_to_internal(OsStr::from_bytes(&raw_bytes));
    assert_eq!(internal, "\u{0}\u{7f}\u{80}\u{fe}\u{ff}");
    let roundtripped = internal_to_platform(&internal);
    assert_eq!(roundtripped.as_bytes(), raw_bytes);
}

/// Java: `testUnicodeToInternal_roundtrip`, same parameters.
#[test]
fn test_unicode_to_internal_roundtrip() {
    for s in ["ascii", "äöüÄÖÜß", "🌱", "羅勒罗勒学名"] {
        let internal = unicode_to_internal(s);
        // In the internal encoding, raw bytes are encoded as Latin-1.
        assert!(is_internal(&internal));
        let roundtripped = internal_to_unicode(&internal);
        if s.is_ascii() {
            assert!(is_same_instance(&internal, s));
            assert!(matches!(roundtripped, Cow::Borrowed(_)));
        } else {
            assert_eq!(roundtripped, s);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Not in the Java test.

#[test]
fn is_internal_is_the_latin1_coder() {
    assert!(is_internal(""));
    assert!(is_internal("abc"));
    assert!(is_internal("\u{80}\u{c3}\u{a9}\u{ff}"));
    assert!(!is_internal("\u{100}"));
    assert!(!is_internal("a€"));
    assert!(!is_internal("🌱"));
}

#[test]
fn latin1_encode_and_decode_are_inverse_on_all_bytes() {
    let all: Vec<u8> = (0..=255).collect();
    let s = latin1_decode(&all);
    assert_eq!(s.chars().count(), 256);
    assert!(is_internal(&s));
    assert_eq!(&*latin1_encode(&s), &all[..]);
    assert!(matches!(latin1_decode(b"abc"), Cow::Borrowed("abc")));
    assert!(matches!(latin1_encode("abc"), Cow::Borrowed(b"abc")));
}

/// Java: StringUnsafe.getInternalStringBytes on a string with the UTF16 coder.
/// A path string that is not internal does not fail at the system boundary: Bazel's JNI layer
/// writes `?` for a char above U+00FF (src/main/native/latin1_jni_path.cc
/// GetStringLatin1Chars, the UTF-16 fallback), as the JDK's ISO-8859-1 encoder does. (For a char
/// outside the BMP the JNI layer writes one `?` per UTF-16 unit; blaze_vfs ports that.)
#[test]
fn internal_to_platform_of_a_string_that_is_not_internal_writes_question_marks() {
    assert_eq!(
        internal_to_platform("a\u{20ac}/\u{e9}").as_bytes(),
        b"a?/\xe9"
    );
}

#[test]
#[should_panic(
    expected = "IllegalArgumentException: Expected internal string with Latin-1 coder, got: a€ ([97, 0, -84, 32])"
)]
fn latin1_encode_rejects_a_string_that_is_not_internal() {
    let _ = latin1_encode("a€");
}

#[test]
#[should_panic(expected = "Expected internal string with Latin-1 coder")]
fn internal_to_unicode_rejects_a_string_that_is_not_internal() {
    let _ = internal_to_unicode("🌱");
}

fn unhex(s: &str) -> Vec<u8> {
    hex::decode(s).unwrap()
}

/// `new String(bytes, UTF_8)` on the oracle's embedded JDK, recorded by
/// `scripts/string-encoding/run` (sample vectors, and digests over every sequence of an alphabet
/// of boundary bytes up to a length).
#[test]
fn java_new_string_utf8_matches_the_jdk() {
    use sha2::{Digest, Sha256};
    let data = include_str!("data/java_utf8_decode_vectors.txt");
    let (mut vectors, mut digests) = (0, 0);
    for line in data.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "#" => {}
            "V" => {
                vectors += 1;
                let got = java_new_string_utf8(&unhex(f[1]));
                assert_eq!(hex::encode(got.as_bytes()), f[2], "input {}", f[1]);
                // internal_to_unicode is the same decoder applied to the internal string's bytes.
                let internal = latin1_decode(&unhex(f[1])).into_owned();
                assert_eq!(hex::encode(internal_to_unicode(&internal).as_bytes()), f[2]);
            }
            "D" => {
                digests += 1;
                let alphabet = unhex(f[1]);
                let max_len: usize = f[2].parse().unwrap();
                let want_count: u64 = f[3].parse().unwrap();
                let mut md = Sha256::new();
                let mut count = 0u64;
                for len in 1..=max_len {
                    let mut idx = vec![0usize; len];
                    let mut seq = vec![0u8; len];
                    'odometer: loop {
                        for i in 0..len {
                            seq[i] = alphabet[idx[i]];
                        }
                        let out = java_new_string_utf8(&seq);
                        md.update((out.len() as u32).to_be_bytes());
                        md.update(out.as_bytes());
                        count += 1;
                        let mut p = len;
                        loop {
                            if p == 0 {
                                break 'odometer;
                            }
                            p -= 1;
                            idx[p] += 1;
                            if idx[p] < alphabet.len() {
                                break;
                            }
                            idx[p] = 0;
                        }
                    }
                }
                assert_eq!(count, want_count);
                assert_eq!(hex::encode(md.finalize()), f[4], "alphabet {}", f[1]);
            }
            other => panic!("unknown line kind {other}"),
        }
    }
    assert_eq!((vectors, digests), (42, 2));
}

// Byte-strings review 2026-11-20 S2-4. Java src/main/native/latin1_jni_path.cc:121-124 (and the
// JDK's getString8859_1Chars) map each UTF-16 unit: <= U+00FF to that byte, others to `?`.
// A patch header `+++ b/🐷.txt` names the file `??.txt` in Bazel.
#[test]
fn jni_latin1_chars_is_per_utf16_unit() {
    use blaze_util::string_encoding::jni_latin1_chars;
    assert_eq!(&*jni_latin1_chars("🐷.txt"), b"??.txt");
    assert_eq!(&*jni_latin1_chars("é中"), b"\xe9?");
    assert_eq!(&*jni_latin1_chars("\u{c3}\u{a9}"), b"\xc3\xa9");
    assert_eq!(&*jni_latin1_chars("abc"), b"abc");
}
