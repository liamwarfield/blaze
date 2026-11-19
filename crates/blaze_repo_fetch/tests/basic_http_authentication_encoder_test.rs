//! Port of `src/test/java/com/google/devtools/build/lib/authandtls/BasicHttpAuthenticationEncoderTest.java`.

use base64::Engine as _;
use blaze_repo_fetch::authandtls::basic_http_authentication_encode;
use blaze_util::string_encoding::{java_new_string_utf8, unicode_to_internal};

/// Java `decode(message)`: base64 after `"Basic "`, decoded as UTF-8, split on the first `:`.
fn decode(message: &str) -> Vec<String> {
    let base64_encoded_message = &message[6..];
    let username_and_password = java_new_string_utf8(
        &base64::engine::general_purpose::STANDARD
            .decode(base64_encoded_message)
            .unwrap(),
    );
    username_and_password
        .splitn(2, ':')
        .map(str::to_string)
        .collect()
}

#[test]
fn encode_normal_username_password_output_expected() {
    let message = basic_http_authentication_encode("Aladdin", "open sesame");
    assert_eq!(message, "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
}

#[test]
fn encode_normal_username_password_can_be_decoded() {
    let message = basic_http_authentication_encode("Aladdin", "open sesame");
    let username_and_password = decode(&message);
    assert_eq!(username_and_password[0], "Aladdin");
    assert_eq!(username_and_password[1], "open sesame");
}

#[test]
fn encode_username_contains_colon_can_be_decoded() {
    let message = basic_http_authentication_encode("foo:user", "foopass");
    let username_and_password = decode(&message);
    assert_eq!(username_and_password[0], "foo");
    assert_eq!(username_and_password[1], "user:foopass");
}

#[test]
fn encode_empty_username_output_expected() {
    let message = basic_http_authentication_encode("", "foopass");
    assert_eq!(message, "Basic OmZvb3Bhc3M=");
}

#[test]
fn encode_empty_password_output_expected() {
    let message = basic_http_authentication_encode("foouser", "");
    assert_eq!(message, "Basic Zm9vdXNlcjo=");
}

#[test]
fn encode_empty_username_password_output_expected() {
    let message = basic_http_authentication_encode("", "");
    assert_eq!(message, "Basic Og==");
}

#[test]
fn encode_special_character_utf8_output_expected() {
    let message = basic_http_authentication_encode("test", &unicode_to_internal("123\u{A3}"));
    assert_eq!(message, "Basic dGVzdDoxMjPCow==");
}
