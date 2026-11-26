//! Port of `GetCredentialsResponseTest`.
// Java: src/test/java/com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsResponseTest.java

use blaze_repo_fetch::authandtls::Headers;
use blaze_repo_fetch::authandtls::credentialhelper::{GetCredentialsResponse, JavaInstant};

fn parse(json: &str) -> GetCredentialsResponse {
    GetCredentialsResponse::parse(json)
        .unwrap_or_else(|e| panic!("{json}: {e}"))
        .unwrap_or_else(|| panic!("{json}: null"))
}

fn assert_throws_json_syntax(json: &str) {
    assert!(
        GetCredentialsResponse::parse(json).is_err(),
        "expected JsonSyntaxException for {json}"
    );
}

fn abc_headers() -> Headers {
    let mut headers = Headers::new();
    headers.insert("a".to_string(), vec![]);
    headers.insert("b".to_string(), vec!["b".to_string()]);
    headers.insert("c".to_string(), vec!["c".to_string(), "c".to_string()]);
    headers
}

/// Truth's `isEqualTo` on the record: `ImmutableMap.equals` ignores order.
fn assert_same_response(actual: &GetCredentialsResponse, expected: &GetCredentialsResponse) {
    assert_eq!(actual.expires(), expected.expires());
    assert_eq!(actual.headers().len(), expected.headers().len());
    for (name, values) in expected.headers() {
        assert_eq!(actual.headers().get(name), Some(values), "header {name}");
    }
}

#[test]
fn parse_valid() {
    assert!(parse("{}").headers().is_empty());
    assert!(parse("{\"headers\": {}}").headers().is_empty());

    let expected_response = GetCredentialsResponse::new(abc_headers(), None);

    assert_same_response(
        &parse("{\"headers\": {\"c\": [\"c\", \"c\"], \"a\": [], \"b\": [\"b\"]}}"),
        &expected_response,
    );
}

#[test]
fn parse_with_extra_fields() {
    assert!(parse("{\"foo\": 123}").headers().is_empty());
    assert!(parse("{\"foo\": 123, \"bar\": []}").headers().is_empty());

    let expected_response = GetCredentialsResponse::new(abc_headers(), None);

    assert_same_response(
        &parse(
            "{\"foo\": 123, \"headers\": {\"c\": [\"c\", \"c\"], \"a\": [], \"b\": [\"b\"]}, \"bar\": 123}",
        ),
        &expected_response,
    );
}

#[test]
fn parse_invalid() {
    assert_throws_json_syntax("[]");
    assert_throws_json_syntax("\"foo\"");
    assert_throws_json_syntax("1");
}

#[test]
fn parse_invalid_headers_envelope() {
    assert_throws_json_syntax("{\"headers\": null}");
    assert_throws_json_syntax("{\"headers\": \"foo\"}");
    assert_throws_json_syntax("{\"headers\": []}");
    assert_throws_json_syntax("{\"headers\": 1}");
}

#[test]
fn parse_invalid_headers_value() {
    assert_throws_json_syntax("{\"headers\": {\"a\": null}}");
    assert_throws_json_syntax("{\"headers\": {\"a\": 1}}");
    assert_throws_json_syntax("{\"headers\": {\"a\": {}}}");
    assert_throws_json_syntax("{\"headers\": {\"a\": \"a\"}}");
    assert_throws_json_syntax("{\"headers\": {\"a\": [null]}}");
    assert_throws_json_syntax("{\"headers\": {\"a\": [\"a\", null]}}");
    assert_throws_json_syntax("{\"headers\": {\"a\": [null, \"a\"]}}");
    assert_throws_json_syntax("{\"headers\": {\"a\": [\"a\", 1]}}");
    assert_throws_json_syntax("{\"headers\": {\"a\": [1, \"a\"]}}");
    assert_throws_json_syntax("{\"headers\": {\"a\": [\"a\", []]}}");
    assert_throws_json_syntax("{\"headers\": {\"a\": [[], \"a\"]}}");
    assert_throws_json_syntax("{\"headers\": {\"a\": [\"a\", {}]}}");
    assert_throws_json_syntax("{\"headers\": {\"a\": [{}, \"a\"]}}");
}

#[test]
fn parse_expires() {
    assert_eq!(
        parse("{\"expires\": \"1970-09-29T11:46:29Z\"}").expires(),
        Some(JavaInstant::of_epoch_second(23456789))
    );
    assert_eq!(
        parse("{\"expires\": \"1970-09-29T11:46:29+00:00\"}").expires(),
        Some(JavaInstant::of_epoch_second(23456789))
    );
    assert_eq!(
        parse("{\"expires\": \"1970-09-29T13:46:29+02:00\"}").expires(),
        Some(JavaInstant::of_epoch_second(23456789))
    );
    assert_eq!(
        parse("{\"expires\": \"1970-09-28T23:46:29-12:00\"}").expires(),
        Some(JavaInstant::of_epoch_second(23456789))
    );
}

#[test]
fn parse_invalid_expires() {
    assert_throws_json_syntax("{\"expires\": null}");
    assert_throws_json_syntax("{\"expires\": \"foo\"}");
    assert_throws_json_syntax("{\"expires\": []}");
    assert_throws_json_syntax("{\"expires\": 1}");
    assert_throws_json_syntax("{\"expires\": {}}");
}

#[test]
fn serialize_empty_headers() {
    let expected_response = GetCredentialsResponse::new(Headers::new(), None);
    assert_eq!(expected_response.to_json(), "{}");
}

#[test]
fn round_trip() {
    let expected_response =
        GetCredentialsResponse::new(abc_headers(), Some(JavaInstant::of_epoch_second(123456789)));

    assert_same_response(&parse(&expected_response.to_json()), &expected_response);
}

// Not a Java test: `ImmutableMap.Builder.build()` on a repeated header name. The text is the one
// the oracle printed for a helper that repeated `Authorization` (scripts: remote-5 probes, see
// docs/reviews/2026-11-19-remote-response.md, Round 5, C1).
#[test]
fn repeated_header_name_is_illegal_argument() {
    use blaze_repo_fetch::authandtls::credentialhelper::get_credentials_response::ResponseParseError;
    assert_eq!(
        GetCredentialsResponse::parse(
            r#"{"headers": {"Authorization": ["Bearer TOKEN"], "Authorization": ["Bearer OTHER"]}}"#
        ),
        Err(ResponseParseError::IllegalArgument(
            "Multiple entries with same key: Authorization=[Bearer OTHER] and Authorization=[Bearer TOKEN]"
                .to_string()
        ))
    );
    // Two "headers" members go into one builder.
    assert_eq!(
        GetCredentialsResponse::parse(r#"{"headers": {"a": ["1", "2"]}, "headers": {"a": []}}"#),
        Err(ResponseParseError::IllegalArgument(
            "Multiple entries with same key: a=[] and a=[1, 2]".to_string()
        ))
    );
    let response = parse(r#"{"headers": {"a": ["1"]}, "headers": {"b": ["2"]}}"#);
    assert_eq!(response.headers().len(), 2);
}
