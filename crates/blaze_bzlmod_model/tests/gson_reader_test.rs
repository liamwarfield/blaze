//! Byte-strings review 2026-11-20 S1-6: the Gson reader port on `\u` escapes and a leading BOM.
//!
//! Expected values from Gson 2.11.0 as bundled in the oracle (`A-server.jar` of bazel 9.3.2, run
//! on its embedded JDK): `new Gson().fromJson(json, Object.class)`. Gson's message also ends with
//! "\nSee https://github.com/google/gson/blob/main/Troubleshooting.md#malformed-json"; the port
//! does not add that line (see `parse_lenient`), so the tests compare the part before it.

use blaze_bzlmod_model::gson::{JsonValue, parse_lenient};

fn error(json: &str) -> String {
    parse_lenient(json).expect_err("must fail").to_string()
}

#[test]
fn unicode_escape_followed_by_multi_byte_char_is_malformed() {
    assert_eq!(
        error("\"\\uabc€\""),
        "com.google.gson.stream.MalformedJsonException: Malformed Unicode escape \\uabc€ at line 1 \
         column 4 path $"
    );
}

#[test]
fn unicode_escape_with_sign_is_malformed() {
    assert_eq!(
        error("\"\\u+123\""),
        "com.google.gson.stream.MalformedJsonException: Malformed Unicode escape \\u+123 at line 1 \
         column 4 path $"
    );
}

#[test]
fn short_unicode_escape_takes_the_following_chars() {
    assert_eq!(
        error("{\"a\": \"\\uab\"}"),
        "com.google.gson.stream.MalformedJsonException: Malformed Unicode escape \\uab\"} at line 1 \
         column 10 path $.a"
    );
}

#[test]
fn unterminated_unicode_escape() {
    assert_eq!(
        error("[\"\\u12"),
        "com.google.gson.stream.MalformedJsonException: Unterminated escape sequence at line 1 \
         column 5 path $[0]"
    );
}

#[test]
fn valid_unicode_escape() {
    assert_eq!(
        parse_lenient("\"\\u00e9x\"").unwrap(),
        Some(JsonValue::String("éx".to_string()))
    );
}

#[test]
fn leading_byte_order_mark_is_skipped() {
    let v = parse_lenient("\u{feff}{\"a\": \"b\"}").unwrap().unwrap();
    let JsonValue::Object(m) = v else {
        panic!("not an object: {v:?}")
    };
    assert_eq!(m.len(), 1);
}

#[test]
fn second_byte_order_mark_is_an_error() {
    assert!(parse_lenient("\u{feff}\u{feff}{\"a\": \"b\"}").is_err());
}
