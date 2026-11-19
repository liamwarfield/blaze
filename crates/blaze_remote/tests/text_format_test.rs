//! Tests of `blaze_remote::text_format` (the reading half of protobuf-java's `TextFormat`).
//!
//! Two sources:
//! - Ported from protobuf-java 33.4 `java/core/src/test/java/com/google/protobuf/TextFormatTest.java`:
//!   the tests of the ported utility functions (`unescapeBytes`/`unescapeText`, `parseInteger`).
//!   PORT: the other `TextFormatTest` methods exercise the printer, `TestAllTypes`, extensions,
//!   `Any` and maps, which Blaze does not port (the parser is driven by the `Platform` schema
//!   only); they are not stubbed here.
//! - Oracle cases: the text of a probe case of `scripts/remote/platform-probe` and the Platform
//!   (or the `ParseException` text inside the ERROR line) that Bazel 9.3.2 produced for it
//!   (`platform-probe oracle td-o1`/`td-o2`). The parser-level order is the parse order; the
//!   oracle prints the sorted properties `PlatformUtils` sends, so the expected lists here are
//!   the oracle's properties in the order the text gives them.

use blaze_remote::reapi::{Platform, platform};
use blaze_remote::text_format::{
    self, parse_int32, parse_int64, parse_uint32, parse_uint64, unescape_bytes, unescape_text,
};

fn property(name: &str, value: &str) -> platform::Property {
    platform::Property {
        name: name.to_string(),
        value: value.to_string(),
    }
}

fn parse(text: &str) -> Result<Vec<platform::Property>, String> {
    let mut builder = Platform::default();
    match text_format::merge_platform(text, &mut builder) {
        Ok(()) => Ok(builder.properties),
        Err(e) => Err(e.get_message().to_string()),
    }
}

// ---------------------------------------------------------------------------------------------
// Ported from TextFormatTest.java.

#[test]
fn test_character_not_in_unicode_block() {
    let actual = unescape_bytes("\\U000358da").unwrap();
    assert_eq!(actual.len(), 4);
}

#[test]
fn test_escape_question_mark() {
    assert_eq!(unescape_text("?").unwrap(), "?");
    assert_eq!(unescape_text("\\?").unwrap(), "?");
}

/// PORT: the `escapeBytes`/`escapeText` halves of this test are not ported (no printer).
#[test]
fn test_escape() {
    // Escape sequences.
    assert_eq!(
        unescape_bytes("\\000\\001\\a\\b\\f\\n\\r\\t\\v\\\\\\'\\\"").unwrap(),
        b"\0\x01\x07\x08\x0c\n\r\t\x0b\\'\"".to_vec()
    );
    assert_eq!(
        unescape_text("\\000\\001\\a\\b\\f\\n\\r\\t\\v\\\\\\'\\\"").unwrap(),
        "\0\u{1}\u{7}\u{8}\u{c}\n\r\t\u{b}\\'\""
    );
    // ESCAPE_TEST_STRING_ESCAPED -> ESCAPE_TEST_STRING
    assert_eq!(
        unescape_text(
            "\\\"A string with \\' characters \\n and \\r newlines and \\t tabs and \\001 \
             slashes \\\\"
        )
        .unwrap(),
        "\"A string with ' characters \n and \r newlines and \t tabs and \u{1} slashes \\"
    );

    // Invariant
    assert_eq!(unescape_bytes("hello").unwrap(), b"hello".to_vec());
    assert_eq!(unescape_text("hello").unwrap(), "hello");

    // Unicode handling.
    assert_eq!(unescape_text("\\341\\210\\264").unwrap(), "\u{1234}");
    assert_eq!(
        unescape_bytes("\\341\\210\\264").unwrap(),
        vec![0xe1, 0x88, 0xb4]
    );
    assert_eq!(unescape_text("\\xe1\\x88\\xb4").unwrap(), "\u{1234}");
    assert_eq!(
        unescape_bytes("\\xe1\\x88\\xb4").unwrap(),
        vec![0xe1, 0x88, 0xb4]
    );
    assert_eq!(unescape_text("\\u1234").unwrap(), "\u{1234}");
    assert_eq!(unescape_bytes("\\u1234").unwrap(), vec![0xe1, 0x88, 0xb4]);
    assert_eq!(
        unescape_bytes("\\U00001234").unwrap(),
        vec![0xe1, 0x88, 0xb4]
    );
    assert_eq!(
        unescape_text("\\xf0\\x90\\x90\\xb7").unwrap(),
        "\u{10437}".to_string()
    );
    assert_eq!(
        unescape_bytes("\\U00010437").unwrap(),
        vec![0xf0, 0x90, 0x90, 0xb7]
    );

    // Handling of strings with unescaped Unicode characters > 255.
    let zh = "\u{9999}\u{6e2f}\u{4e0a}\u{6d77}\u{23f80}\u{8c50}\u{9280}\u{884c}";
    assert_eq!(unescape_bytes(zh).unwrap(), zh.as_bytes().to_vec());

    // Errors.
    assert!(unescape_text("\\x").is_err());
    assert!(unescape_text("\\z").is_err());
    assert!(unescape_text("\\").is_err());

    let message = |s: &str| unescape_text(s).unwrap_err().0;
    assert_eq!(
        message("\\u"),
        "Invalid escape sequence: '\\u' with too few hex chars"
    );
    assert_eq!(
        message("\\ud800"),
        "Invalid escape sequence: '\\u' refers to a surrogate"
    );
    assert_eq!(
        message("\\ud800\\u1234"),
        "Invalid escape sequence: '\\u' refers to a surrogate"
    );
    assert_eq!(
        message("\\udc00"),
        "Invalid escape sequence: '\\u' refers to a surrogate"
    );
    assert_eq!(
        message("\\ud801\\udc37"),
        "Invalid escape sequence: '\\u' refers to a surrogate"
    );
    assert_eq!(
        message("\\U1234"),
        "Invalid escape sequence: '\\U' with too few hex chars"
    );
    assert_eq!(
        message("\\U1234no more hex"),
        "Invalid escape sequence: '\\U' with too few hex chars"
    );
    assert_eq!(
        message("\\U00110000"),
        "Invalid escape sequence: '\\U00110000' is not a valid code point value"
    );
    assert_eq!(
        message("\\U0000d801\\U00000dc37"),
        "Invalid escape sequence: '\\U0000d801' refers to a surrogate code unit"
    );
}

#[test]
fn test_parse_integer() {
    assert_eq!(parse_int32("0").unwrap(), 0);
    assert_eq!(parse_int32("1").unwrap(), 1);
    assert_eq!(parse_int32("-1").unwrap(), -1);
    assert_eq!(parse_int32("12345").unwrap(), 12345);
    assert_eq!(parse_int32("-12345").unwrap(), -12345);
    assert_eq!(parse_int32("2147483647").unwrap(), 2147483647);
    assert_eq!(parse_int32("-2147483648").unwrap(), -2147483648);

    assert_eq!(parse_uint32("0").unwrap(), 0);
    assert_eq!(parse_uint32("1").unwrap(), 1);
    assert_eq!(parse_uint32("12345").unwrap(), 12345);
    assert_eq!(parse_uint32("2147483647").unwrap(), 2147483647);
    assert_eq!(parse_uint32("2147483648").unwrap(), 2147483648u32 as i32);
    assert_eq!(parse_uint32("4294967295").unwrap(), 4294967295u32 as i32);

    assert_eq!(parse_int64("0").unwrap(), 0);
    assert_eq!(parse_int64("1").unwrap(), 1);
    assert_eq!(parse_int64("-1").unwrap(), -1);
    assert_eq!(parse_int64("12345").unwrap(), 12345);
    assert_eq!(parse_int64("-12345").unwrap(), -12345);
    assert_eq!(parse_int64("2147483647").unwrap(), 2147483647);
    assert_eq!(parse_int64("-2147483648").unwrap(), -2147483648);
    assert_eq!(parse_int64("4294967295").unwrap(), 4294967295);
    assert_eq!(parse_int64("4294967296").unwrap(), 4294967296);
    assert_eq!(
        parse_int64("9223372036854775807").unwrap(),
        9223372036854775807
    );
    assert_eq!(
        parse_int64("-9223372036854775808").unwrap(),
        -9223372036854775808
    );

    assert_eq!(parse_uint64("0").unwrap(), 0);
    assert_eq!(parse_uint64("1").unwrap(), 1);
    assert_eq!(parse_uint64("12345").unwrap(), 12345);
    assert_eq!(parse_uint64("2147483647").unwrap(), 2147483647);
    assert_eq!(parse_uint64("4294967295").unwrap(), 4294967295);
    assert_eq!(parse_uint64("4294967296").unwrap(), 4294967296);
    assert_eq!(
        parse_uint64("9223372036854775807").unwrap(),
        9223372036854775807
    );
    assert_eq!(
        parse_uint64("9223372036854775808").unwrap(),
        -9223372036854775808
    );
    assert_eq!(parse_uint64("18446744073709551615").unwrap(), -1);

    // Hex
    assert_eq!(parse_int32("0x1234abcd").unwrap(), 0x1234abcd);
    assert_eq!(parse_int32("-0x1234abcd").unwrap(), -0x1234abcd);
    assert_eq!(parse_uint64("0xffffffffffffffff").unwrap(), -1);
    assert_eq!(
        parse_int64("0x7fffffffffffffff").unwrap(),
        0x7fffffffffffffff
    );

    // Octal
    assert_eq!(parse_int32("01234567").unwrap(), 0o1234567);

    // Out-of-range
    assert!(parse_int32("2147483648").is_err());
    assert!(parse_int32("-2147483649").is_err());
    assert!(parse_uint32("4294967296").is_err());
    assert!(parse_uint32("-1").is_err());
    assert!(parse_int64("9223372036854775808").is_err());
    assert!(parse_int64("-9223372036854775809").is_err());
    assert!(parse_uint64("18446744073709551616").is_err());
    assert!(parse_uint64("-1").is_err());

    // Not a number.
    assert!(parse_int32("abcd").is_err());
}

// ---------------------------------------------------------------------------------------------
// Oracle cases (scripts/remote/platform-probe).

/// Case `one`.
#[test]
fn oracle_one() {
    assert_eq!(
        parse("properties { name: \"image\" value: \"docker://x\" }\n"),
        Ok(vec![property("image", "docker://x")])
    );
}

/// Case `colon`: `properties: {}` adds an empty Property (the oracle sends `=` first).
#[test]
fn oracle_colon() {
    assert_eq!(
        parse("properties: { name: \"a\" value: \"1\" }\nproperties: {}\n"),
        Ok(vec![property("a", "1"), property("", "")])
    );
}

/// Case `escapes`: every escape of `unescapeBytes`, single quotes, adjacent literals.
#[test]
fn oracle_escapes() {
    assert_eq!(
        parse(
            "properties { name: \"esc\" value: \
             \"a\\tb\\nc\\x41\\101\\u00e9\\U0001F600\\'\\\"\\\\\\?\\a\\v\\f\\r\\b\\0\" }\n\
             properties { name: 'single' value: 'it''s' \"joined\" }\n"
        ),
        Ok(vec![
            property(
                "esc",
                "a\tb\ncAA\u{e9}\u{1F600}'\"\\?\u{7}\u{b}\u{c}\r\u{8}\0"
            ),
            property("single", "itsjoined"),
        ])
    );
}

/// Case `comment`.
#[test]
fn oracle_comment() {
    assert_eq!(
        parse(
            "# leading comment\nproperties { # inside\n  name: \"a\" # after name\n  value: \"1\"\n\
             } # trailing\n"
        ),
        Ok(vec![property("a", "1")])
    );
}

/// Case `angle-separators`: `<`...`>` and `,`/`;` separators.
#[test]
fn oracle_angle_separators() {
    assert_eq!(
        parse("properties < name: 'b'; value: 'x', >;properties { name: 'a', value: 'y'; },\n"),
        Ok(vec![property("b", "x"), property("a", "y")])
    );
}

/// Case `list`: the short repeated form, and `[]`.
#[test]
fn oracle_list() {
    assert_eq!(
        parse(
            "properties: [ { name: \"b\" value: \"2\" }, < name: \"a\" value: \"1\" > ]\n\
             properties: []\n"
        ),
        Ok(vec![property("b", "2"), property("a", "1")])
    );
}

/// Case `duplicate-name`: a singular field given twice keeps the last value
/// (ALLOW_SINGULAR_OVERWRITES); the oracle then sends `b=2` (a later name wins in
/// `PlatformUtils`).
#[test]
fn oracle_duplicate_name() {
    assert_eq!(
        parse(
            "properties { name: \"a\" name: \"b\" value: \"1\" }\nproperties { name: \"b\" value: \"2\" }\n"
        ),
        Ok(vec![property("b", "1"), property("b", "2")])
    );
}

/// Case `unknown-field`.
#[test]
fn oracle_unknown_field() {
    assert_eq!(
        parse("properties { name: \"a\" value: \"1\" }\nfoo: \"x\"\n"),
        Err("2:1: Input contains unknown fields and/or extensions:\n\
             2:1:\tbuild.bazel.remote.execution.v2.Platform.foo"
            .to_string())
    );
}

/// Case `unknown-nested`: unknown fields are skipped (message, float, hex, short repeated,
/// identifier values), an extension is unknown too, and all are listed.
#[test]
fn oracle_unknown_nested() {
    assert_eq!(
        parse(
            "properties { name: \"a\" value: \"1\" other { x: 1.5e3 y: -0x10 z: [1, 2] w: inf } }\n\
             [ext.field]: 3\n"
        ),
        Err("1:35: Input contains unknown fields and/or extensions:\n\
             1:35:\tbuild.bazel.remote.execution.v2.Platform.Property.other\n\
             2:6:\tbuild.bazel.remote.execution.v2.Platform.[ext.field]"
            .to_string())
    );
}

/// Case `broken`.
#[test]
fn oracle_broken() {
    assert_eq!(
        parse("properties { name \"a\" }\n"),
        Err("1:19: Expected \":\".".to_string())
    );
}

/// Case `broken-string`.
#[test]
fn oracle_broken_string() {
    assert_eq!(
        parse("properties {\n  name: \"a\\q\" }\n"),
        Err("2:9: Invalid escape sequence: '\\q'".to_string())
    );
}

/// Case `broken-eof`.
#[test]
fn oracle_broken_eof() {
    assert_eq!(
        parse("properties { name: \"a\"\n"),
        Err("2:1: Expected \"}\".".to_string())
    );
}

/// Case `broken-ident`: `42` is an identifier to `consumeIdentifier`.
#[test]
fn oracle_broken_ident() {
    assert_eq!(
        parse("properties { name: \"a\" } 42: \"x\"\n"),
        Err("1:26: Input contains unknown fields and/or extensions:\n\
             1:26:\tbuild.bazel.remote.execution.v2.Platform.42"
            .to_string())
    );
}

/// Case `unterminated`: the string token ends at the end of the line.
#[test]
fn oracle_unterminated() {
    assert_eq!(
        parse("properties { name: \"a }\n"),
        Err("1:20: String missing ending quote.".to_string())
    );
}

/// Case `invalid-value`: an unknown scalar field whose value is not a value.
#[test]
fn oracle_invalid_value() {
    assert_eq!(
        parse("foo: , properties { name: \"a\" value: \"1\" }\n"),
        Err("1:6: Invalid field value: ,".to_string())
    );
}

/// Case `bad-unicode`.
#[test]
fn oracle_bad_unicode() {
    assert_eq!(
        parse("properties { name: \"a\" value: \"\\ud800\" }\n"),
        Err("1:31: Invalid escape sequence: '\\u' refers to a surrogate".to_string())
    );
}
