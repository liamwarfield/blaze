//! Port of `src/test/java/com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsResponseTest.java`.

use blaze_repo_fetch::authandtls::credentialhelper::{
    GetCredentialsResponse, Instant, JsonSyntaxException, catch_unchecked,
};

fn from_json(json: &str) -> Result<Option<GetCredentialsResponse>, JsonSyntaxException> {
    GetCredentialsResponse::from_json(json)
}

fn parse(json: &str) -> GetCredentialsResponse {
    from_json(json).unwrap().unwrap()
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}

fn expected_abc() -> GetCredentialsResponse {
    let mut expected_response_builder = GetCredentialsResponse::new_builder();
    expected_response_builder.headers_builder_put("a", strings(&[]));
    expected_response_builder.headers_builder_put("b", strings(&["b"]));
    expected_response_builder.headers_builder_put("c", strings(&["c", "c"]));
    expected_response_builder.build()
}

#[test]
fn parse_valid() {
    assert!(parse("{}").headers().is_empty());
    assert!(parse("{\"headers\": {}}").headers().is_empty());

    assert_eq!(
        parse("{\"headers\": {\"c\": [\"c\", \"c\"], \"a\": [], \"b\": [\"b\"]}}"),
        expected_abc()
    );
}

#[test]
fn parse_with_extra_fields() {
    assert!(parse("{\"foo\": 123}").headers().is_empty());
    assert!(parse("{\"foo\": 123, \"bar\": []}").headers().is_empty());

    assert_eq!(
        parse(
            "{\"foo\": 123, \"headers\": {\"c\": [\"c\", \"c\"], \"a\": [], \"b\": [\"b\"]}, \"bar\": 123}"
        ),
        expected_abc()
    );
}

#[test]
fn parse_invalid() {
    assert!(from_json("[]").is_err());
    assert!(from_json("\"foo\"").is_err());
    assert!(from_json("1").is_err());
}

#[test]
fn parse_invalid_headers_envelope() {
    assert!(from_json("{\"headers\": null}").is_err());
    assert!(from_json("{\"headers\": \"foo\"}").is_err());
    assert!(from_json("{\"headers\": []}").is_err());
    assert!(from_json("{\"headers\": 1}").is_err());
}

#[test]
fn parse_invalid_headers_value() {
    for json in [
        "{\"headers\": {\"a\": null}}",
        "{\"headers\": {\"a\": 1}}",
        "{\"headers\": {\"a\": {}}}",
        "{\"headers\": {\"a\": \"a\"}}",
        "{\"headers\": {\"a\": [null]}}",
        "{\"headers\": {\"a\": [\"a\", null]}}",
        "{\"headers\": {\"a\": [null, \"a\"]}}",
        "{\"headers\": {\"a\": [\"a\", 1]}}",
        "{\"headers\": {\"a\": [1, \"a\"]}}",
        "{\"headers\": {\"a\": [\"a\", []]}}",
        "{\"headers\": {\"a\": [[], \"a\"]}}",
        "{\"headers\": {\"a\": [\"a\", {}]}}",
        "{\"headers\": {\"a\": [{}, \"a\"]}}",
    ] {
        assert!(from_json(json).is_err(), "{json}");
    }
}

#[test]
fn parse_expires() {
    assert_eq!(
        parse("{\"expires\": \"1970-09-29T11:46:29Z\"}").expires(),
        Some(Instant::of_epoch_second(23456789))
    );
    assert_eq!(
        parse("{\"expires\": \"1970-09-29T11:46:29+00:00\"}").expires(),
        Some(Instant::of_epoch_second(23456789))
    );
    assert_eq!(
        parse("{\"expires\": \"1970-09-29T13:46:29+02:00\"}").expires(),
        Some(Instant::of_epoch_second(23456789))
    );
    assert_eq!(
        parse("{\"expires\": \"1970-09-28T23:46:29-12:00\"}").expires(),
        Some(Instant::of_epoch_second(23456789))
    );
}

#[test]
fn parse_invalid_expires() {
    assert!(from_json("{\"expires\": null}").is_err());
    assert!(from_json("{\"expires\": \"foo\"}").is_err());
    assert!(from_json("{\"expires\": []}").is_err());
    assert!(from_json("{\"expires\": 1}").is_err());
    assert!(from_json("{\"expires\": {}}").is_err());
}

#[test]
fn serialize_empty_headers() {
    let expected_response = GetCredentialsResponse::new_builder().build();
    assert_eq!(expected_response.to_json(), "{}");
}

#[test]
fn round_trip() {
    let mut expected_response_builder = GetCredentialsResponse::new_builder();
    expected_response_builder.set_expires(Instant::of_epoch_second(123456789));
    expected_response_builder.headers_builder_put("a", strings(&[]));
    expected_response_builder.headers_builder_put("b", strings(&["b"]));
    expected_response_builder.headers_builder_put("c", strings(&["c", "c"]));
    let expected_response = expected_response_builder.build();

    assert_eq!(parse(&expected_response.to_json()), expected_response);
}

// The tests below have no Java counterpart: they pin the adapters' messages, Gson's output and
// the RFC 3339 formatter's strict parsing and lenient resolving.

#[test]
fn serialize_writes_gson_bytes() {
    let mut builder = GetCredentialsResponse::new_builder();
    builder.set_expires(Instant::of_epoch_second(123456789));
    builder.headers_builder_put("a", strings(&[]));
    builder.headers_builder_put("b", strings(&["b", "x=y"]));
    assert_eq!(
        builder.build().to_json(),
        "{\"headers\":{\"a\":[],\"b\":[\"b\",\"x\\u003dy\"]},\"expires\":\"1973-11-29T21:33:09Z\"}"
    );
}

#[test]
fn parse_errors_carry_the_adapter_messages() {
    let message = |json: &str| from_json(json).unwrap_err().message;
    assert_eq!(message("1"), "Expected object, got NUMBER");
    assert_eq!(
        message("{\"headers\": []}"),
        "Expected value of 'headers' to be an object, got BEGIN_ARRAY"
    );
    assert_eq!(
        message("{\"headers\": {\"a\": \"a\"}}"),
        "Expected value of 'a' header to be an array of strings, got STRING"
    );
    assert_eq!(
        message("{\"headers\": {\"a\": [\"a\", null]}}"),
        "Expected value 1 of 'a' header to be a string, got NULL"
    );
    assert_eq!(
        message("{\"expires\": 1}"),
        "Expected value of 'expires' to be a string, got NUMBER"
    );
    assert_eq!(
        message("{\"expires\": \"foo\"}"),
        "Expected value of 'expires' to be a RFC 3339 formatted timestamp: Text 'foo' could not be parsed at index 0"
    );
    assert_eq!(
        message("{\"expires\": \"1970-09-29T11:46:29.5Z\"}"),
        "Expected value of 'expires' to be a RFC 3339 formatted timestamp: Text '1970-09-29T11:46:29.5Z' could not be parsed at index 19"
    );
    assert_eq!(message("{} {}"), "JSON document was not fully consumed.");
}

#[test]
fn parse_expires_resolves_leniently() {
    // February 30th is March 2nd; 24:00:00 is midnight of the next day.
    assert_eq!(
        parse("{\"expires\": \"2023-02-30T00:00:00Z\"}").expires(),
        parse("{\"expires\": \"2023-03-02T00:00:00Z\"}").expires()
    );
    assert_eq!(
        parse("{\"expires\": \"2023-03-01T24:00:00Z\"}").expires(),
        parse("{\"expires\": \"2023-03-02T00:00:00Z\"}").expires()
    );
    assert_eq!(
        parse("{\"expires\": \"2023-13-01T00:00:00Z\"}").expires(),
        parse("{\"expires\": \"2024-01-01T00:00:00Z\"}").expires()
    );
}

#[test]
fn duplicate_header_names_are_an_illegal_argument() {
    let e = catch_unchecked(|| from_json("{\"headers\": {\"a\": [\"1\"], \"a\": [\"2\"]}}"))
        .unwrap_err();
    assert_eq!(e.get_class_name(), "java.lang.IllegalArgumentException");
    assert_eq!(
        e.get_message(),
        "Multiple entries with same key: a=[1] and a=[2]"
    );
}

#[test]
fn a_leading_byte_order_mark_is_skipped() {
    assert_eq!(
        parse("\u{feff}{\"headers\": {\"a\": [\"b\"]}}"),
        parse("{\"headers\": {\"a\": [\"b\"]}}")
    );
    assert_eq!(from_json("\u{feff}"), Ok(None));
}

/// `new Gson().fromJson` reads leniently (Gson's `JsonReader` with `Strictness.LENIENT`).
#[test]
fn lenient_syntax_is_read_like_gson() {
    let expected = parse("{\"headers\": {\"a\": [\"b\", \"c\"]}}");
    for json in [
        "{'headers': {'a': ['b', 'c']}}",
        "{headers: {a: [b, c]}}",
        "{\"headers\" = {\"a\" => [\"b\"; \"c\"]}}",
        "{\"foo\": 1; \"headers\": {\"a\": [\"b\", \"c\"]}}",
        "/* c */ {\"headers\": {\"a\": [\"b\", \"c\"]}} // c",
        "# c\n{\"headers\": {\"a\": [\"b\", \"c\"]} # c\n}",
        ")]}'\n{\"headers\": {\"a\": [\"b\", \"c\"]}}",
        "{\"foo\": [1,,2,], \"bar\": NaN, \"baz\": TRUE, \"headers\": {\"a\": [\"b\", \"c\"]}}",
    ] {
        assert_eq!(parse(json), expected, "{json}");
    }
    // Unquoted literals are strings, so they pass the adapter's type checks.
    assert_eq!(
        parse("{\"headers\": {\"a\": [01, NaN]}}"),
        parse("{\"headers\": {\"a\": [\"01\", \"NaN\"]}}")
    );
    // A missing array element is null.
    assert_eq!(
        from_json("{\"headers\": {\"a\": [\"b\",]}}")
            .unwrap_err()
            .message,
        "Expected value 1 of 'a' header to be a string, got NULL"
    );
    // A document of comments only is empty.
    assert_eq!(from_json("# nothing\n// here\n"), Ok(None));
    // An unquoted top-level literal is a string, which the adapter rejects with its own message.
    assert_eq!(
        from_json("not json").unwrap_err().message,
        "Expected object, got STRING"
    );
}

#[test]
fn standard_syntax_errors_stay_syntax_errors() {
    for json in [
        "{\"headers\": {}",
        "{\"headers\" {}}",
        "{\"foo\": 1,}",
        "{\"foo\": 1 \"bar\": 2}",
        "{\"foo\": }",
        "{\"foo\": \"bar}",
        "{\"foo\": [1 2]}",
        "{,}",
    ] {
        assert!(from_json(json).is_err(), "{json}");
    }
}
