//! Port of `com.google.devtools.build.lib.util.StringEncoding` and of the parts of
//! `com.google.devtools.build.lib.unsafe.StringUnsafe` it is built on.
//!
//! Utility functions for reencoding strings between Bazel's internal raw byte encoding and
//! regular (Unicode) strings.
//!
//! The three kinds of strings of the Java class documentation, in Rust (decisions.md 2026-11-18
//! "Strings", docs/research/07-byte-strings.md option A):
//!
//! - **Internal strings**: all strings retained by Blaze and used in its inner layers are raw byte
//!   sequences stored one byte per `char`: a `str` whose chars are all `<= U+00FF` (Java: a
//!   `String` with the `LATIN1` coder). [`is_internal`] is the Rust form of
//!   `StringUnsafe.getCoder(s) == LATIN1`.
//! - **Unicode strings**: regular Rust strings (a `string` field of a protobuf message, JSON).
//! - **Platform strings**: what is passed to or returned from the operating system (file names,
//!   argv, environment). In Rust these are byte strings (`OsStr`).
//!
//! PORT: Java's platform strings are Java `String`s that the JVM encodes with `sun.jnu.encoding`.
//! The Bazel client forces the `en_US.ISO-8859-1` locale when the host has it (`blaze.cc:1388`),
//! and then "platform strings are identical to internal strings and Java (N)IO functions can be
//! used to operate with Unix API on a raw byte level". Blaze calls the Unix API with bytes itself,
//! so it always has that raw-byte behaviour: [`internal_to_platform`] and
//! [`platform_to_internal`] are the Latin-1 encoder and decoder. The other Java branch (no Latin-1
//! locale: platform strings are a subset of Unicode strings, so file names that are not UTF-8 are
//! mangled by the JVM) is not reproduced.
//!
//! All functions are the identity on ASCII and return the borrowed input then (Java returns the
//! same instance).
//!
//! # Migration seam
//!
//! Until Starlark sources, file names and command-line input are decoded as Latin-1 (migration
//! steps 2 and 3 of docs/research/07-byte-strings.md), the strings Blaze's inner layers hold are
//! real Unicode, not internal strings. [`STRINGS_ARE_INTERNAL`] says which of the two holds, and
//! the four seam functions ([`to_internal`], [`internal_string_bytes`],
//! [`internal_string_from_bytes`], [`internal_length`]) give, for either state, what Java computes
//! from its internal string. Code that calls them is right before the switch (for every string
//! that is valid UTF-8 text) and after it (for every string). Afterwards each seam function is
//! the plain function named in its documentation and can be replaced by it.
//!
//! The decoding itself is on the seam too: the source decoders (`StarlarkUtil.createParserInput`
//! for BUILD, .bzl, MODULE.bazel, REPO.bazel, VENDOR.bazel) must produce their text with
//! [`parser_input_from_latin1`], which reads the same constant as the fingerprint bridge, so
//! that the two cannot be switched separately. `tests/string_seam_guard_test.rs` fails when a
//! decoder and the constant disagree.
//!
//! ## The switch
//!
//! 1. Before the switch, with no change of behaviour (every step is the identity while the
//!    constant is `false`):
//!    - both `create_parser_input` functions (`blaze_sky_loading/src/starlark_util.rs`,
//!      `blaze_bzlmod/src/module_file/compiled_module_file.rs`) call
//!      [`parser_input_from_latin1`];
//!    - every place where a string leaves or enters as bytes calls [`internal_string_bytes`] or
//!      [`internal_string_from_bytes`] instead of `as_bytes()` / `String::from_utf8*`: process
//!      argv, environment and working directory, file system paths and directory entries, the
//!      console and event output, file-writing actions, repository rule `read` / `execute` /
//!      `file` / `template`, command-line and rc-file intake, and the query, cquery and aquery
//!      writers (the Java list is docs/research/07-byte-strings.md 2.5);
//!    - every `String.length()` or char index that Java computes on an internal string calls
//!      [`internal_length`];
//!    - text that really is Unicode (protos, JSON, lockfile, registry files) is converted with
//!      [`unicode_to_internal`] / [`internal_to_unicode`] where Java calls `StringEncoding`.
//! 2. The interpreter's part: starlark-rust escapes, case mapping, `strip`, `repr`, and Blaze's
//!    `json.decode` (07-byte-strings.md section 4, option A).
//! 3. Set [`STRINGS_ARE_INTERNAL`] to `true`. This is the only edit of the switch itself.
//! 4. Check: `scripts/cargo-capped test -p blaze_util -p blaze_actions` (the guard, and
//!    `non_ascii_action_keys` / `oracle_aquery_dump`, which run both sides);
//!    `scripts/actions-probe/run --binary <blaze> --out <file>` and `cmp <file>
//!    crates/blaze_actions/tests/data/non_ascii_probe/aquery.jsonproto` (all 78 actions,
//!    byte for byte, including the four `"\377"` probes); `scripts/bytes/run --binary <blaze>`;
//!    the gate (brotli and re2 stay byte-identical).
//! 5. Later: delete the seam (each function becomes the plain one), the
//!    `Fingerprint::add_internal_string` alias and the before-the-switch halves of the tests.

use std::borrow::Cow;
use std::cell::Cell;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};

/// Whether the strings Blaze holds where Java holds internal strings (Starlark strings, paths,
/// options, environment) are internal strings yet. `false`: they are still real Unicode (sources
/// are read with `String::from_utf8`). See "Migration seam" in the module comment.
// PORT: transitional; delete with the seam functions once it has been `true` for a while.
pub const STRINGS_ARE_INTERNAL: bool = false;

thread_local! {
    static STRINGS_ARE_INTERNAL_OVERRIDE: Cell<Option<bool>> = const { Cell::new(None) };
}

/// [`STRINGS_ARE_INTERNAL`], or the value a test set for this thread.
pub fn strings_are_internal() -> bool {
    STRINGS_ARE_INTERNAL_OVERRIDE
        .get()
        .unwrap_or(STRINGS_ARE_INTERNAL)
}

/// Runs `f` with [`strings_are_internal`] forced to `value` on this thread, so that tests can
/// check both sides of the seam. Not for production code.
pub fn with_strings_are_internal_for_testing<R>(value: bool, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<bool>);
    impl Drop for Restore {
        fn drop(&mut self) {
            STRINGS_ARE_INTERNAL_OVERRIDE.set(self.0);
        }
    }
    let _restore = Restore(STRINGS_ARE_INTERNAL_OVERRIDE.replace(Some(value)));
    f()
}

/// Seam: the internal string that `s` stands for. After the switch: `s` itself.
///
/// Use it where Java applies a Unicode API to an internal string without converting it (the
/// UTF-8 encoders behind `Fingerprint.addString`, `String.getBytes(UTF_8)`,
/// `Hashing.hashString(s, UTF_8)`): the UTF-8 of the result is what Java encodes.
pub fn to_internal(s: &str) -> Cow<'_, str> {
    if s.is_ascii() || strings_are_internal() {
        Cow::Borrowed(s)
    } else {
        unicode_to_internal(s)
    }
}

/// Seam: `StringUnsafe.getInternalStringBytes(s)` / `s.getBytes(ISO_8859_1)` of the internal
/// string that `s` stands for. After the switch: [`latin1_encode`].
pub fn internal_string_bytes(s: &str) -> Cow<'_, [u8]> {
    if s.is_ascii() || !strings_are_internal() {
        Cow::Borrowed(s.as_bytes())
    } else {
        latin1_encode(s)
    }
}

/// Seam: `new String(bytes, ISO_8859_1)` / `StringUnsafe.newInstance(bytes, LATIN1)`, as the
/// string Blaze holds for that internal string. After the switch: [`latin1_decode`], which never
/// fails. Before it, bytes that are not UTF-8 have no string and are an error; a caller that
/// cannot report one calls `unported!`.
pub fn internal_string_from_bytes(bytes: &[u8]) -> Result<Cow<'_, str>, std::str::Utf8Error> {
    if bytes.is_ascii() || strings_are_internal() {
        Ok(latin1_decode(bytes))
    } else {
        std::str::from_utf8(bytes).map(Cow::Borrowed)
    }
}

/// Seam: Java `ParserInput.fromLatin1(bytes, file)`: the text of a Starlark source file (BUILD,
/// .bzl, MODULE.bazel, REPO.bazel, VENDOR.bazel) as the interpreter gets it. **Every source
/// decoder calls this and nothing else**, so that Starlark strings are internal strings exactly
/// when [`strings_are_internal`] says so, which is what `Fingerprint::add_string` and the other
/// seam functions assume. After the switch: [`latin1_decode`] (one char per byte, never fails).
/// Before it, a file that is not UTF-8 has no text and is an error; Java accepts such a file
/// unless `--incompatible_enforce_starlark_utf8=error`, so the caller reports it or calls
/// `unported!`.
// Java: net/starlark/java/syntax/ParserInput.java ParserInput.fromLatin1
// Java: com/google/devtools/build/lib/skyframe/StarlarkUtil.java:43 StarlarkUtil.createParserInput
pub fn parser_input_from_latin1(bytes: &[u8]) -> Result<Cow<'_, str>, std::str::Utf8Error> {
    internal_string_from_bytes(bytes)
}

/// Seam: `String.length()` of the internal string that `s` stands for (its number of bytes).
/// After the switch: `s.chars().count()`.
pub fn internal_length(s: &str) -> usize {
    if s.is_ascii() || !strings_are_internal() {
        s.len()
    } else {
        s.chars().count()
    }
}

/// Whether `s` can be an internal string: every char is `<= U+00FF`.
///
/// Java: com/google/devtools/build/lib/unsafe/StringUnsafe.java:67 StringUnsafe.getCoder
/// (`== LATIN1`; with compact strings a Java `String` has the Latin-1 coder exactly when all its
/// chars are `<= U+00FF`).
pub fn is_internal(s: &str) -> bool {
    // A char <= U+00FF is ASCII or the two bytes C2/C3 xx; every char above it starts with a
    // lead byte >= 0xC4.
    s.bytes().all(|b| b < 0xc4)
}

/// Decodes bytes as ISO-8859-1: one char per byte. The result is an internal string.
///
/// Java: com/google/devtools/build/lib/unsafe/StringUnsafe.java:128 StringUnsafe.newInstance
/// (`newInstance(bytes, LATIN1)`), and `new String(bytes, ISO_8859_1)`.
pub fn latin1_decode(bytes: &[u8]) -> Cow<'_, str> {
    if bytes.is_ascii() {
        // ASCII is valid UTF-8.
        return Cow::Borrowed(std::str::from_utf8(bytes).expect("ASCII"));
    }
    Cow::Owned(bytes.iter().map(|&b| b as char).collect())
}

/// The bytes of an internal string: one byte per char.
///
/// Java: com/google/devtools/build/lib/unsafe/StringUnsafe.java:88
/// StringUnsafe.getInternalStringBytes. As in Java, a string that is not internal (a char above
/// U+00FF: "constructed out of a non-ASCII string that hasn't been converted to internal
/// encoding") is a crash with Java's message.
pub fn latin1_encode(s: &str) -> Cow<'_, [u8]> {
    if s.is_ascii() {
        return Cow::Borrowed(s.as_bytes());
    }
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        let c = c as u32;
        if c > 0xff {
            not_internal(s);
        }
        out.push(c as u8);
    }
    Cow::Owned(out)
}

/// Java: StringUnsafe.java:92 `throw new IllegalArgumentException(String.format("Expected internal
/// string with Latin-1 coder, got: %s (%s)", truncatedString, Arrays.toString(getByteArray(...))))`.
fn not_internal(s: &str) -> ! {
    // Guava Ascii.truncate(obj, 1000, "..."), in UTF-16 units as Java counts them.
    let mut units: Vec<u16> = s.encode_utf16().collect();
    if units.len() > 1000 {
        units.truncate(997);
        units.extend("...".encode_utf16());
    }
    let truncated = String::from_utf16_lossy(&units);
    // The byte array of a UTF16-coder string, in native (little-endian) order, as signed bytes.
    let bytes: Vec<String> = units
        .iter()
        .flat_map(|u| u.to_le_bytes())
        .map(|b| (b as i8).to_string())
        .collect();
    panic!(
        "IllegalArgumentException: Expected internal string with Latin-1 coder, got: {} ([{}])",
        truncated,
        bytes.join(", ")
    );
}

/// Transforms an internal string into a platform string as efficiently as possible.
///
/// Java: com/google/devtools/build/lib/util/StringEncoding.java:104
/// StringEncoding.internalToPlatform
// PORT: the Linux, Latin-1 locale branch of `needsReencodeForPlatform` (see the module comment):
// the platform string is the internal string's bytes.
pub fn internal_to_platform(s: &str) -> Cow<'_, OsStr> {
    match latin1_encode(s) {
        Cow::Borrowed(b) => Cow::Borrowed(OsStr::from_bytes(b)),
        Cow::Owned(b) => Cow::Owned(OsString::from_vec(b)),
    }
}

/// Transforms a platform string into an internal string as efficiently as possible.
///
/// Java: com/google/devtools/build/lib/util/StringEncoding.java:115
/// StringEncoding.platformToInternal
// PORT: the Linux, Latin-1 locale branch, as for `internal_to_platform`.
pub fn platform_to_internal(s: &OsStr) -> Cow<'_, str> {
    latin1_decode(s.as_bytes())
}

/// Transforms an internal string into a Unicode string as efficiently as possible.
///
/// Java: com/google/devtools/build/lib/util/StringEncoding.java:126
/// StringEncoding.internalToUnicode (`new String(getInternalStringBytes(s), UTF_8)`: byte
/// sequences that are not UTF-8 become U+FFFD the way the JDK's decoder replaces them).
pub fn internal_to_unicode(s: &str) -> Cow<'_, str> {
    if !needs_reencode_for_unicode(s) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(java_new_string_utf8(&latin1_encode(s)))
}

/// Transforms a Unicode string into an internal string as efficiently as possible.
///
/// Java: com/google/devtools/build/lib/util/StringEncoding.java:137
/// StringEncoding.unicodeToInternal (`newInstance(s.getBytes(UTF_8), LATIN1)`)
// PORT: Java's `getBytes(UTF_8)` writes `?` for an unpaired surrogate; a Rust `str` cannot hold
// one, so the UTF-8 bytes are the string's own.
pub fn unicode_to_internal(s: &str) -> Cow<'_, str> {
    if !needs_reencode_for_unicode(s) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(s.bytes().map(|b| b as char).collect())
}

/// Java: StringEncoding.java:158 StringEncoding.needsReencodeForUnicode
/// (`!StringUnsafe.isAscii(s)`)
fn needs_reencode_for_unicode(s: &str) -> bool {
    !s.is_ascii()
}

/// `new String(bytes, UTF_8)`: the JDK's replacing UTF-8 decoder. One U+FFFD per malformed
/// sequence, with the JDK's own notion of how long a malformed sequence is (it differs from
/// `String::from_utf8_lossy`, for example for an encoded surrogate, which is one U+FFFD in Java
/// and three in Rust).
///
/// Java (JDK): java.base/java/lang/String.java String.decodeUTF8_UTF16 with `doReplace = true`.
/// The oracle's embedded JDK is the reference; `tests/string_encoding_test.rs` holds vectors
/// recorded from it.
pub fn java_new_string_utf8(src: &[u8]) -> String {
    const REPL: char = '\u{fffd}';
    // Java works on signed bytes; the predicates below are written on `u8`.
    fn is_not_continuation(b: u8) -> bool {
        b & 0xc0 != 0x80
    }
    fn is_malformed3(b1: u8, b2: u8, b3: u8) -> bool {
        (b1 == 0xe0 && b2 & 0xe0 == 0x80) || b2 & 0xc0 != 0x80 || b3 & 0xc0 != 0x80
    }
    fn is_malformed3_2(b1: u8, b2: u8) -> bool {
        (b1 == 0xe0 && b2 & 0xe0 == 0x80) || b2 & 0xc0 != 0x80
    }
    fn is_malformed4(b2: u8, b3: u8, b4: u8) -> bool {
        b2 & 0xc0 != 0x80 || b3 & 0xc0 != 0x80 || b4 & 0xc0 != 0x80
    }
    fn is_malformed4_2(b1: u8, b2: u8) -> bool {
        (b1 == 0xf0 && !(0x90..=0xbf).contains(&b2))
            || (b1 == 0xf4 && b2 & 0xf0 != 0x80)
            || b2 & 0xc0 != 0x80
    }
    fn is_malformed4_3(b3: u8) -> bool {
        b3 & 0xc0 != 0x80
    }
    fn malformed3(src: &[u8], sp: usize) -> usize {
        let b1 = src[sp];
        let b2 = src[sp + 1]; // no need to lookup b3
        if (b1 == 0xe0 && b2 & 0xe0 == 0x80) || is_not_continuation(b2) {
            1
        } else {
            2
        }
    }
    fn malformed4(src: &[u8], sp: usize) -> usize {
        // we don't care the speed here
        let b1 = src[sp];
        let b2 = src[sp + 1];
        if b1 > 0xf4
            || (b1 == 0xf0 && !(0x90..=0xbf).contains(&b2))
            || (b1 == 0xf4 && b2 & 0xf0 != 0x80)
            || is_not_continuation(b2)
        {
            return 1;
        }
        if is_not_continuation(src[sp + 2]) {
            return 2;
        }
        3
    }

    let sl = src.len();
    let mut sp = 0;
    let mut dst = String::with_capacity(sl);
    while sp < sl {
        let b1 = src[sp];
        sp += 1;
        if b1 < 0x80 {
            dst.push(b1 as char);
        } else if b1 >> 5 == 0b110 && b1 & 0x1e != 0 {
            if sp < sl {
                let b2 = src[sp];
                sp += 1;
                if is_not_continuation(b2) {
                    dst.push(REPL);
                    sp -= 1;
                } else {
                    let c = (((b1 & 0x1f) as u32) << 6) | (b2 & 0x3f) as u32;
                    dst.push(char::from_u32(c).expect("two-byte sequence"));
                }
                continue;
            }
            // underflow()
            dst.push(REPL);
            break;
        } else if b1 >> 4 == 0b1110 {
            if sp + 1 < sl {
                let b2 = src[sp];
                let b3 = src[sp + 1];
                sp += 2;
                if is_malformed3(b1, b2, b3) {
                    dst.push(REPL);
                    sp -= 3;
                    sp += malformed3(src, sp);
                } else {
                    let c = (((b1 & 0x0f) as u32) << 12)
                        | (((b2 & 0x3f) as u32) << 6)
                        | (b3 & 0x3f) as u32;
                    // Character.isSurrogate(c)
                    match char::from_u32(c) {
                        None => dst.push(REPL),
                        Some(c) => dst.push(c),
                    }
                }
                continue;
            }
            if sp < sl && is_malformed3_2(b1, src[sp]) {
                dst.push(REPL);
                continue;
            }
            dst.push(REPL);
            break;
        } else if b1 >> 3 == 0b11110 {
            if sp + 2 < sl {
                let b2 = src[sp];
                let b3 = src[sp + 1];
                let b4 = src[sp + 2];
                sp += 3;
                let uc = (((b1 & 0x07) as u32) << 18)
                    | (((b2 & 0x3f) as u32) << 12)
                    | (((b3 & 0x3f) as u32) << 6)
                    | (b4 & 0x3f) as u32;
                // shortest form check: Character.isSupplementaryCodePoint(uc)
                if is_malformed4(b2, b3, b4) || !(0x1_0000..=0x10_ffff).contains(&uc) {
                    dst.push(REPL);
                    sp -= 4;
                    sp += malformed4(src, sp);
                } else {
                    dst.push(char::from_u32(uc).expect("supplementary code point"));
                }
                continue;
            }
            if b1 > 0xf4 || (sp < sl && is_malformed4_2(b1, src[sp])) {
                dst.push(REPL);
                continue;
            }
            sp += 1;
            dst.push(REPL);
            if sp < sl && is_malformed4_3(src[sp]) {
                continue;
            }
            break;
        } else {
            dst.push(REPL);
        }
    }
    dst
}
