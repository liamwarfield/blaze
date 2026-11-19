//! Port of `src/test/java/com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsRequestTest.java`.

use blaze_repo_fetch::authandtls::credentialhelper::{GetCredentialsRequest, JsonSyntaxException};
use blaze_repo_fetch::uri::JavaUri;

fn from_json(json: &str) -> Result<Option<GetCredentialsRequest>, JsonSyntaxException> {
    GetCredentialsRequest::from_json(json)
}

fn uri_of(json: &str) -> JavaUri {
    from_json(json).unwrap().unwrap().uri().clone()
}

fn uri(s: &str) -> JavaUri {
    JavaUri::create(s).unwrap()
}

#[test]
fn parse_valid() {
    assert_eq!(
        uri_of("{\"uri\": \"http://example.com\"}"),
        uri("http://example.com")
    );
    assert_eq!(
        uri_of("{\"uri\": \"https://example.com\"}"),
        uri("https://example.com")
    );
    assert_eq!(
        uri_of("{\"uri\": \"grpc://example.com\"}"),
        uri("grpc://example.com")
    );
    assert_eq!(
        uri_of("{\"uri\": \"grpcs://example.com\"}"),
        uri("grpcs://example.com")
    );

    assert_eq!(
        uri_of("{\"uri\": \"uri-without-protocol\"}"),
        uri("uri-without-protocol")
    );
}

#[test]
fn parse_missing_uri() {
    assert!(from_json("{}").is_err());
    assert!(from_json("{\"foo\": 1}").is_err());
    assert!(from_json("{\"foo\": 1, \"bar\": 2}").is_err());
}

#[test]
fn parse_non_string_uri() {
    assert!(from_json("[]").is_err());
    assert!(from_json("\"foo\"").is_err());
    assert!(from_json("1").is_err());
    assert!(from_json("{\"uri\": 1}").is_err());
    assert!(from_json("{\"uri\": {}}").is_err());
    assert!(from_json("{\"uri\": []}").is_err());
    assert!(from_json("{\"uri\": [\"https://example.com\"]}").is_err());
    assert!(from_json("{\"uri\": null}").is_err());
}

#[test]
fn parse_with_extra_fields() {
    assert_eq!(
        uri_of("{\"uri\": \"http://example.com\", \"foo\": 1}"),
        uri("http://example.com")
    );
    assert_eq!(
        uri_of("{\"foo\": 1, \"uri\": \"http://example.com\"}"),
        uri("http://example.com")
    );
    assert_eq!(
        uri_of("{\"uri\": \"http://example.com\", \"foo\": 1, \"bar\": {}}"),
        uri("http://example.com")
    );
    assert_eq!(
        uri_of("{\"foo\": 1, \"uri\": \"http://example.com\", \"bar\": []}"),
        uri("http://example.com")
    );
}

// The tests below have no Java counterpart: they pin the adapters' messages and the bytes Gson
// writes on the helper's stdin.

#[test]
fn parse_errors_carry_the_adapter_messages() {
    assert_eq!(
        from_json("[]").unwrap_err().message,
        "Expected object, got BEGIN_ARRAY"
    );
    assert_eq!(
        from_json("{\"uri\": 1}").unwrap_err().message,
        "Expected value of 'url' to be a string, got NUMBER"
    );
    assert_eq!(
        from_json("{}").unwrap_err().message,
        "java.lang.IllegalStateException: Missing required properties: uri"
    );
}

#[test]
fn empty_document_and_null_are_null() {
    assert_eq!(from_json(""), Ok(None));
    assert_eq!(from_json(" \n"), Ok(None));
    assert_eq!(from_json("null"), Ok(None));
}

#[test]
fn serialize_is_html_safe_like_gson() {
    let request = GetCredentialsRequest::new_builder()
        .set_uri(uri("https://example.com/a?b=c&d='e'"))
        .build()
        .unwrap();
    assert_eq!(
        request.to_json(),
        "{\"uri\":\"https://example.com/a?b\\u003dc\\u0026d\\u003d\\u0027e\\u0027\"}"
    );
    assert_eq!(
        GetCredentialsRequest::from_json(&request.to_json()),
        Ok(Some(request))
    );
}
