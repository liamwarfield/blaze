//! How protobuf-java decodes malformed UTF-8 in `string` values, against vectors recorded from
//! Bazel 9.3.2's own server jar on the oracle's embedded JDK (`scripts/protobuf-utf8/run`,
//! `tests/data/protobuf_utf8_vectors.txt`). Every site is `new String(bytes, UTF_8)`:
//! `TextFormat.Tokenizer.consumeString` and `TextFormat.unescapeText` (`ByteString.toStringUtf8`),
//! `CodedInputStream.readString` for a proto2 string, and `readStringRequireUtf8` (an error) for
//! a proto3 one; the printer prints the decoded `String`.

use blaze_protos::JavaMessage;
use blaze_protos::blaze_query::License;
use blaze_protos::build::bazel::remote::execution::v2::OutputFile;
use blaze_protos::descriptors::pool;
use blaze_protos::dynamic_message::{DynamicMessage, Value};
use blaze_protos::java::new_string_utf8;
use blaze_protos::text_format::{self, printer, unescape_text};

fn unhex(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn hex(b: &[u8]) -> String {
    if b.is_empty() {
        return "-".to_string();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn octal(b: &[u8]) -> String {
    b.iter().map(|x| format!("\\{x:03o}")).collect()
}

fn field1(b: &[u8]) -> Vec<u8> {
    let mut out = vec![0x0a, b.len() as u8];
    out.extend_from_slice(b);
    out
}

fn parse_output_file(b: &[u8]) -> OutputFile {
    text_format::parse::<OutputFile>(&format!("path: \"{}\"", octal(b))).unwrap()
}

fn actual(kind: &str, b: &[u8]) -> String {
    match kind {
        "parse" => hex(parse_output_file(b).path.as_bytes()),
        "print" => hex(printer().print_to_string(&parse_output_file(b)).as_bytes()),
        "print_raw" => hex(
            printer()
                .escaping_non_ascii(false)
                .print_to_string(&parse_output_file(b))
                .as_bytes(),
        ),
        "unescape_text" => hex(unescape_text(&octal(b)).unwrap().as_bytes()),
        "dynamic_proto2" => {
            let d = pool()
                .find_message_type_by_name(License::FULL_NAME)
                .unwrap();
            let m = DynamicMessage::parse_from(d, &field1(b)).unwrap();
            match &m.get_values(d.find_field_by_number(1).unwrap())[0] {
                Value::String(s) => hex(s),
                v => panic!("not a string: {v:?}"),
            }
        }
        "dynamic_proto3" => {
            let d = pool()
                .find_message_type_by_name(OutputFile::FULL_NAME)
                .unwrap();
            match DynamicMessage::parse_from(d, &field1(b)) {
                Ok(m) => match m.get_field(d.find_field_by_number(1).unwrap()) {
                    Some(Value::String(s)) => hex(s),
                    None => "-".to_string(),
                    v => panic!("not a string: {v:?}"),
                },
                Err(e) => format!("ERR:{}", e.to_string().replace(' ', "_")),
            }
        }
        _ => panic!("unknown kind {kind}"),
    }
}

#[test]
fn protobuf_string_decoding_matches_java() {
    let data = include_str!("data/protobuf_utf8_vectors.txt");
    let mut n = 0;
    for line in data.lines() {
        if line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split(' ').collect();
        let input = unhex(f[0]);
        assert_eq!(actual(f[1], &input), f[2], "{} of {}", f[1], f[0]);
        n += 1;
    }
    assert_eq!(n, 258);
}

/// The review's case (`docs/reviews/2026-11-20-byte-strings.md` S2-3): an encoded surrogate is
/// one U+FFFD in Java, three with `String::from_utf8_lossy`.
#[test]
fn encoded_surrogate_is_one_replacement_char() {
    assert_eq!(new_string_utf8(b"\xed\xa0\x80"), "\u{fffd}");
    assert_eq!(parse_output_file(b"\xed\xa0\x80").path, "\u{fffd}");
}
