//! Tests of `blaze_protos::text_format` (port of protobuf-java 33.4 `TextFormat`).
//!
//! Two sources:
//! - Oracle goldens (`tests/data/text`, written by `scripts/record-text-goldens` from Bazel
//!   9.3.2): `--build_event_binary_file` and `--build_event_text_file` of one build, cquery
//!   `--output=proto` and `--output=textproto`, aquery `--output=textproto`. The printer must
//!   reproduce Bazel's text from Bazel's binary output byte for byte, the parser must read
//!   Bazel's text back to the same messages, and printing those again must give Bazel's bytes.
//! - Ported from `java/core/src/test/java/com/google/protobuf/TextFormatTest.java` (same
//!   release). Most of its tests use protobuf's own test protos (`proto2_unittest.TestAllTypes`,
//!   `TestMap`, `TestRecursiveMessage`, ...), which are edition-2023 files that Blaze does not
//!   compile; those tests are `#[ignore]`d. PORT: a few string tests use
//!   `build.bazel.remote.execution.v2.OutputFile` (`string path = 1`) in place of
//!   `TestAllTypes.optional_string`; the unknown-field tests use
//!   `build.bazel.remote.execution.v2.Digest` (no fields 5, 8 or 15) in place of
//!   `TestEmptyMessage`.

use blaze_protos::JavaMessage;
use prost::Message as _;
use blaze_protos::analysis::CqueryResult;
use blaze_protos::build::bazel::remote::execution::v2::{Digest, OutputFile};
use blaze_protos::build_event_stream::BuildEvent;
use blaze_protos::descriptors::pool;
use blaze_protos::dynamic_message::{DynamicMessage, UnknownField, UnknownFieldSet};
use blaze_protos::text_format::{
    self, InvalidEscapeSequenceException, default_format_printer, escape_bytes,
    escape_double_quotes_and_backslashes, escape_text, get_parser, parse_int32, parse_int64,
    parse_uint32, parse_uint64, printer, unescape_bytes, unescape_text,
};

const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/text");

fn read(name: &str) -> Vec<u8> {
    std::fs::read(format!("{DATA}/{name}")).unwrap()
}

fn read_text(name: &str) -> String {
    String::from_utf8(read(name)).expect("TextFormat output is ASCII")
}

// ---------------------------------------------------------------------------------------------
// Oracle goldens.

/// `writeDelimitedTo` records.
fn delimited<M: JavaMessage>(mut data: &[u8]) -> Vec<M> {
    let mut out = Vec::new();
    while let Some(m) = blaze_protos::java::parse_delimited_from::<M>(&mut data).unwrap() {
        out.push(m);
    }
    out
}

/// `--build_event_text_file` of the same build as `--build_event_binary_file`: each event is
/// `TextFormatFileTransport.serializeEvent` of it.
#[test]
fn oracle_bep_text_file_is_printer_output_of_binary_file() {
    let events: Vec<BuildEvent> = delimited(&read("bep.pb"));
    let mut text = String::new();
    for event in &events {
        text.push_str("event {\n");
        text.push_str(&printer().print_to_string(event));
        text.push_str("}\n\n");
    }
    assert_eq!(text, read_text("bep.txt"));
}

/// Each event of the text file parses back to the event of the binary file.
#[test]
fn oracle_bep_text_file_parses_back() {
    let events: Vec<BuildEvent> = delimited(&read("bep.pb"));
    let text = read_text("bep.txt");
    let blocks: Vec<&str> = text
        .strip_suffix("}\n\n")
        .unwrap()
        .split("}\n\nevent {\n")
        .collect();
    assert_eq!(blocks.len(), events.len());
    for (i, (block, event)) in blocks.iter().zip(&events).enumerate() {
        let block = if i == 0 {
            block.strip_prefix("event {\n").unwrap()
        } else {
            block
        };
        let parsed: BuildEvent = text_format::parse(block).unwrap();
        assert!(
            DynamicMessage::from_message(&parsed).java_equals(&DynamicMessage::from_message(event)),
            "event {i}"
        );
    }
}

/// cquery `--output=textproto` is `TextFormat.printer().print` of each message
/// `--output=proto` writes (a `CqueryResult` holding one result or configuration).
#[test]
fn oracle_cquery_textproto_is_printer_output_of_proto() {
    let data = read("cquery.pb");
    // The binary output is the concatenation of single-field CqueryResults: one message.
    let merged = CqueryResult::decode(data.as_slice()).unwrap();
    let mut text = String::new();
    for result in &merged.results {
        let one = CqueryResult {
            results: vec![result.clone()],
            ..Default::default()
        };
        printer().print(&one, &mut text);
    }
    for configuration in &merged.configurations {
        let one = CqueryResult {
            configurations: vec![configuration.clone()],
            ..Default::default()
        };
        printer().print(&one, &mut text);
    }
    assert_eq!(text, read_text("cquery.txt"));

    // And the whole text parses back to the merged message.
    let parsed: CqueryResult = text_format::parse(&read_text("cquery.txt")).unwrap();
    assert!(
        DynamicMessage::from_message(&parsed).java_equals(&DynamicMessage::from_message(&merged))
    );
}

/// aquery `--output=textproto` prints `<field> {\n` + `message.toString()` + `}\n` for each
/// message of the `ActionGraphContainer`; parsing each and printing it again gives the same
/// bytes.
#[test]
fn oracle_aquery_textproto_reprints_identically() {
    let text = read_text("aquery.txt");
    let container = pool()
        .find_message_type_by_name("analysis.ActionGraphContainer")
        .unwrap();
    let mut records: Vec<(String, String)> = Vec::new();
    for line in text.split_inclusive('\n') {
        match line.strip_suffix(" {\n") {
            Some(label)
                if container.find_field_by_name(label).is_some()
                    && records.last().is_none_or(|(_, b)| b.ends_with("}\n")) =>
            {
                records.push((label.to_string(), String::new()))
            }
            _ => records.last_mut().unwrap().1.push_str(line),
        }
    }
    assert!(!records.is_empty());
    let mut out = String::new();
    for (label, body) in &records {
        let field = container.find_field_by_name(label).unwrap();
        let mut m = DynamicMessage::new(field.get_message_type());
        get_parser()
            .merge_dynamic(body.strip_suffix("}\n").unwrap(), &mut m)
            .unwrap();
        out.push_str(&format!(
            "{label} {{\n{}}}\n",
            default_format_printer().print_dynamic_to_string(&m)
        ));
    }
    assert_eq!(out, text);
}

// ---------------------------------------------------------------------------------------------
// Ported from TextFormatTest.java.

fn make_unknown_field_set() -> UnknownFieldSet {
    let mut inner = UnknownFieldSet::default();
    inner.fields.insert(
        12,
        UnknownField {
            varint: vec![6],
            ..Default::default()
        },
    );
    let mut group = UnknownFieldSet::default();
    group.fields.insert(
        10,
        UnknownField {
            varint: vec![5],
            ..Default::default()
        },
    );
    let mut set = UnknownFieldSet::default();
    set.fields.insert(
        5,
        UnknownField {
            varint: vec![1],
            fixed32: vec![2],
            fixed64: vec![3],
            length_delimited: vec![b"4".to_vec(), inner.to_byte_array()],
            group: vec![group],
        },
    );
    set.fields.insert(
        8,
        UnknownField {
            varint: vec![1, 2, 3],
            ..Default::default()
        },
    );
    set.fields.insert(
        15,
        UnknownField {
            varint: vec![0xABCDEF1234567890u64 as i64],
            fixed32: vec![0xABCD1234u32 as i32],
            fixed64: vec![0xABCDEF1234567890u64 as i64],
            ..Default::default()
        },
    );
    set
}

#[test]
fn test_print_unknown_fields() {
    // Test printing of unknown fields in a message.
    let descriptor = pool().find_message_type_by_name(Digest::FULL_NAME).unwrap();
    let message =
        DynamicMessage::parse_from(descriptor, &make_unknown_field_set().to_byte_array()).unwrap();

    assert_eq!(
        printer().print_dynamic_to_string(&message),
        "5: 1\n\
         5: 0x00000002\n\
         5: 0x0000000000000003\n\
         5: \"4\"\n\
         5: {\n\
         \x20 12: 6\n\
         }\n\
         5 {\n\
         \x20 10: 5\n\
         }\n\
         8: 1\n\
         8: 2\n\
         8: 3\n\
         15: 12379813812177893520\n\
         15: 0xabcd1234\n\
         15: 0xabcdef1234567890\n"
    );
}

#[test]
fn test_short_debug_string_unknown() {
    assert_eq!(
        printer()
            .emitting_single_line(true)
            .print_unknown_fields_to_string(&make_unknown_field_set()),
        "5: 1 5: 0x00000002 5: 0x0000000000000003 5: \"4\" 5: { 12: 6 } 5 { 10: 5 } 8: 1 8: 2 \
         8: 3 15: 12379813812177893520 15: 0xabcd1234 15: 0xabcdef1234567890"
    );
}

#[test]
fn test_print_to_unicode_string_unknown() {
    let mut set = UnknownFieldSet::default();
    set.fields.insert(
        1,
        UnknownField {
            length_delimited: vec![vec![0xe3, 0x81, 0x82]],
            ..Default::default()
        },
    );
    assert_eq!(
        printer()
            .escaping_non_ascii(false)
            .print_unknown_fields_to_string(&set),
        "1: \"\\343\\201\\202\"\n"
    );
}

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

const ESCAPE_TEST_STRING: &str =
    "\"A string with ' characters \n and \r newlines and \t tabs and \u{1} slashes \\";
const ESCAPE_TEST_STRING_ESCAPED: &str =
    "\\\"A string with \\' characters \\n and \\r newlines and \\t tabs and \\001 slashes \\\\";

fn message(input: &str) -> String {
    match unescape_text(input) {
        Err(InvalidEscapeSequenceException(m)) => m,
        Ok(v) => panic!("Should have thrown an exception; got {v:?}"),
    }
}

#[test]
fn test_escape() {
    // Escape sequences.
    assert_eq!(
        escape_bytes(b"\0\x01\x07\x08\x0c\n\r\t\x0b\\'\"\x7f"),
        "\\000\\001\\a\\b\\f\\n\\r\\t\\v\\\\\\'\\\"\\177"
    );
    assert_eq!(
        escape_text("\0\u{1}\u{7}\u{8}\u{c}\n\r\t\u{b}\\'\"\u{7f}"),
        "\\000\\001\\a\\b\\f\\n\\r\\t\\v\\\\\\'\\\"\\177"
    );
    assert_eq!(
        unescape_bytes("\\000\\001\\a\\b\\f\\n\\r\\t\\v\\\\\\'\\\"").unwrap(),
        b"\0\x01\x07\x08\x0c\n\r\t\x0b\\'\"".to_vec()
    );
    assert_eq!(
        unescape_text("\\000\\001\\a\\b\\f\\n\\r\\t\\v\\\\\\'\\\"").unwrap(),
        "\0\u{1}\u{7}\u{8}\u{c}\n\r\t\u{b}\\'\""
    );
    assert_eq!(escape_text(ESCAPE_TEST_STRING), ESCAPE_TEST_STRING_ESCAPED);
    assert_eq!(
        unescape_text(ESCAPE_TEST_STRING_ESCAPED).unwrap(),
        ESCAPE_TEST_STRING
    );

    // Invariant
    assert_eq!(escape_bytes(b"hello"), "hello");
    assert_eq!(escape_text("hello"), "hello");
    assert_eq!(unescape_bytes("hello").unwrap(), b"hello".to_vec());
    assert_eq!(unescape_text("hello").unwrap(), "hello");

    // Unicode handling.
    assert_eq!(escape_text("\u{1234}"), "\\341\\210\\264");
    assert_eq!(escape_bytes(&[0xe1, 0x88, 0xb4]), "\\341\\210\\264");
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
    assert_eq!(unescape_text("\\xf0\\x90\\x90\\xb7").unwrap(), "\u{10437}");
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

    // escapeDoubleQuotesAndBackslashes (TextFormat.printToUnicodeString's escaping).
    assert_eq!(
        escape_double_quotes_and_backslashes("a\\b\"c"),
        "a\\\\b\\\"c"
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

#[test]
fn test_parse_string() {
    let zh = "\u{9999}\u{6e2f}\u{4e0a}\u{6d77}\u{23f80}\u{8c50}\u{9280}\u{884c}";
    let mut builder = OutputFile::default();
    text_format::merge(&format!("path: \"{zh}\""), &mut builder).unwrap();
    assert_eq!(builder.path, zh);
}

#[test]
fn test_parse_long_string() {
    let long_text = "123456789012345678901234567890123456789012345678901234567890".repeat(20);
    let mut builder = OutputFile::default();
    text_format::merge(&format!("path: \"{long_text}\""), &mut builder).unwrap();
    assert_eq!(builder.path, long_text);
}

#[test]
fn test_parse_adjacent_string_literals() {
    let mut builder = OutputFile::default();
    text_format::merge("path: \"foo\" 'corge' \"grault\"", &mut builder).unwrap();
    assert_eq!(builder.path, "foocorgegrault");
}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_message() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_message_builder() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_extensions() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_field() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_repeated_field_using_short_repeated_primitives_uses_regular_notation_for_message_type()
 {
}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_repeated_field_using_short_repeated_primitives_uses_short_notation_for_primitive_type()
 {
}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_exotic() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_roundtrip_proto3_optional() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_message_set() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_merge() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_merge_initialized() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_initialized() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_merge_uninitialized() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_uninitialized() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_merge_reader() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_merge_extensions() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_extensions() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_merge_and_parse_compatibility() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_merge_and_parse_exotic() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_merge_message_set() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_merge_message_set_with_overwrite_forbidden() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_merge_numeric_enum() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_merge_angle_brackets() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_merge_comment() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_any_custom_built_type_registry() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_any_any_with_dynamic_message() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_any_any_with_dynamic_message_containing_extension_treated_as_unknown() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_any_any_with_dynamic_message_containing_extension_with_registry() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_any_any_from_with_no_value_field() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_any_any_from_with_no_type_url_field() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_any_any_with_invalid_field_type() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_merge_any_custom_built_type_registry() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_errors() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_boolean() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_field_value() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_field_value_throws() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_short_debug_string() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_short_debug_string_field() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_to_unicode_string() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_to_unicode_string_with_newlines() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_unknown_extensions() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_unknown_extension_with_any_message() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_non_repeated_fields() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_short_repeated_form_of_repeated_fields() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_short_repeated_form_of_empty_repeated_fields() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_short_repeated_form_with_trailing_comma() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_short_repeated_form_of_non_repeated_fields() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_group_like_delimited() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_group_like_delimited_extension() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_group_like_not_delimited() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_group_like_mismatched_name() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_group_like_extension_mismatched_name() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_group_like_mismatched_scope() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_group_like_extension_mismatched_scope() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_print_group_like_mismatched_file() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_delimited_group_like_type() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_delimited_group_like_field() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_delimited_group_like_extension() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_delimited_group_like_invalid() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_delimited_group_like_invalid_extension() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_delimited() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_delimited_invalid() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_parse_delimited_invalid_scope() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_oneof_text_format() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_oneof_overwrite_forbidden() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_oneof_overwrite_allowed() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_map_text_format() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_map_duplicate_keys() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_map_short_form() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_map_short_form_empty() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_map_short_form_trailing_comma() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_map_overwrite() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_map_dynamic_message() {}

#[test]
#[ignore = "tests MapEntryAdapter (private) with TestMap entries; needs map_unittest.proto"]
fn test_map_key_adapter_comparison() {}

#[test]
#[ignore = "tests MapEntryAdapter (private) with TestMap entries; needs map_unittest.proto"]
fn test_map_key_adapter_comparison_invalid_type() {}

#[test]
#[ignore = "TextFormatParseInfoTree is not ported (no Bazel caller)"]
fn test_parse_info_tree_building() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_sort_map_fields() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_preserves_floating_point_negative0() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_default_recursion_limit() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_recursion_limit_with_unknown_fields() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_recursion_limit_with_known_and_unknown_fields() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_recursion_limit_with_any() {}

#[test]
#[ignore = "needs the protobuf-java test protos (proto2_unittest.TestAllTypes and friends: edition-2023 files; the descriptor pool holds the proto2/proto3 files Blaze compiles)"]
fn test_recursion_limit_with_top_level_any() {}
