//! Port of `GetCredentialsRequestTest`.
// Java: src/test/java/com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsRequestTest.java

use blaze_repo_fetch::authandtls::credentialhelper::GetCredentialsRequest;
use blaze_repo_fetch::uri::JavaUri;

fn parse_uri(json: &str) -> JavaUri {
    GetCredentialsRequest::parse(json)
        .unwrap_or_else(|e| panic!("{json}: {e}"))
        .unwrap_or_else(|| panic!("{json}: null"))
        .uri()
        .clone()
}

fn uri(s: &str) -> JavaUri {
    JavaUri::create(s).unwrap()
}

fn assert_throws_json_syntax(json: &str) {
    assert!(
        GetCredentialsRequest::parse(json).is_err(),
        "expected JsonSyntaxException for {json}"
    );
}

#[test]
fn parse_valid() {
    assert_eq!(
        parse_uri("{\"uri\": \"http://example.com\"}"),
        uri("http://example.com")
    );
    assert_eq!(
        parse_uri("{\"uri\": \"https://example.com\"}"),
        uri("https://example.com")
    );
    assert_eq!(
        parse_uri("{\"uri\": \"grpc://example.com\"}"),
        uri("grpc://example.com")
    );
    assert_eq!(
        parse_uri("{\"uri\": \"grpcs://example.com\"}"),
        uri("grpcs://example.com")
    );

    assert_eq!(
        parse_uri("{\"uri\": \"uri-without-protocol\"}"),
        uri("uri-without-protocol")
    );
}

#[test]
fn parse_missing_uri() {
    assert_throws_json_syntax("{}");
    assert_throws_json_syntax("{\"foo\": 1}");
    assert_throws_json_syntax("{\"foo\": 1, \"bar\": 2}");
}

#[test]
fn parse_non_string_uri() {
    assert_throws_json_syntax("[]");
    assert_throws_json_syntax("\"foo\"");
    assert_throws_json_syntax("1");
    assert_throws_json_syntax("{\"uri\": 1}");
    assert_throws_json_syntax("{\"uri\": {}}");
    assert_throws_json_syntax("{\"uri\": []}");
    assert_throws_json_syntax("{\"uri\": [\"https://example.com\"]}");
    assert_throws_json_syntax("{\"uri\": null}");
}

#[test]
fn parse_with_extra_fields() {
    assert_eq!(
        parse_uri("{\"uri\": \"http://example.com\", \"foo\": 1}"),
        uri("http://example.com")
    );
    assert_eq!(
        parse_uri("{\"foo\": 1, \"uri\": \"http://example.com\"}"),
        uri("http://example.com")
    );
    assert_eq!(
        parse_uri("{\"uri\": \"http://example.com\", \"foo\": 1, \"bar\": {}}"),
        uri("http://example.com")
    );
    assert_eq!(
        parse_uri("{\"foo\": 1, \"uri\": \"http://example.com\", \"bar\": []}"),
        uri("http://example.com")
    );
}
