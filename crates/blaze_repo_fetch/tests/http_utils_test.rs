//! Port of `HttpUtilsTest`.

use blaze_repo_fetch::downloader::http_utils::{get_extension, get_location};
use blaze_repo_fetch::uri::JavaUri;

fn uri(s: &str) -> JavaUri {
    JavaUri::create(s).unwrap()
}

#[test]
fn get_extension_two_extensions_returns_last() {
    assert_eq!(get_extension("doodle.tar.gz"), "gz");
}

#[test]
fn get_extension_is_uppercase_returns_lowered() {
    assert_eq!(get_extension("DOODLE.TXT"), "txt");
}

#[test]
fn get_location_missing_in_redirect_throws_io_exception() {
    assert!(get_location(None, &uri("http://lol.example")).is_err());
}

#[test]
fn get_location_absolute_in_redirect_returns_new_url() {
    assert_eq!(
        get_location(Some("http://new.example/hi"), &uri("http://lol.example")).unwrap(),
        uri("http://new.example/hi")
    );
}

#[test]
fn get_location_redirect_only_has_path_merges_host_from_original_url() {
    assert_eq!(
        get_location(Some("/hi"), &uri("http://lol.example")).unwrap(),
        uri("http://lol.example/hi")
    );
}

#[test]
fn get_location_only_has_path_without_slash_fails_to_merge() {
    let e = get_location(Some("omg"), &uri("http://lol.example")).unwrap_err();
    assert!(e.get_message().contains("Could not merge"), "{}", e);
}

#[test]
fn get_location_has_fragment_prefers_new_fragment() {
    assert_eq!(
        get_location(
            Some("http://new.example/hi#b"),
            &uri("http://lol.example#a")
        )
        .unwrap(),
        uri("http://new.example/hi#b")
    );
}

#[test]
fn get_location_has_no_fragment_but_original_does_merges_old_fragment() {
    assert_eq!(
        get_location(Some("http://new.example/hi"), &uri("http://lol.example#a")).unwrap(),
        uri("http://new.example/hi#a")
    );
}

#[test]
fn get_location_old_url_has_pass_redirecting_to_same_domain_merges_password() {
    assert_eq!(
        get_location(
            Some("http://lol.example/hi"),
            &uri("http://a:b@lol.example")
        )
        .unwrap(),
        uri("http://a:b@lol.example/hi")
    );
    assert_eq!(
        get_location(Some("/hi"), &uri("http://a:b@lol.example")).unwrap(),
        uri("http://a:b@lol.example/hi")
    );
}

#[test]
fn get_location_old_url_has_password_redirecting_to_new_server_doesnt_merge() {
    assert_eq!(
        get_location(
            Some("http://new.example/hi"),
            &uri("http://a:b@lol.example")
        )
        .unwrap(),
        uri("http://new.example/hi")
    );
    assert_eq!(
        get_location(
            Some("http://lol.example:81/hi"),
            &uri("http://a:b@lol.example")
        )
        .unwrap(),
        uri("http://lol.example:81/hi")
    );
}

#[test]
fn get_location_redirect_to_ftp_throws_io_exception() {
    let e = get_location(Some("ftp://lol.example"), &uri("http://lol.example")).unwrap_err();
    assert!(e.get_message().contains("Bad Location"), "{}", e);
}

#[test]
fn get_location_redirect_to_https_works() {
    assert_eq!(
        get_location(Some("https://lol.example"), &uri("http://lol.example")).unwrap(),
        uri("https://lol.example")
    );
}

#[test]
fn get_location_preserves_quoting_if_not_inheriting() {
    let redirect = "http://redirected.example.org/foo?response-content-disposition=attachment%3Bfilename%3D%22bar.tar.gz%22";
    let result = get_location(Some(redirect), &uri("http://original.example.org")).unwrap();
    assert_eq!(result, uri(redirect));
    assert_eq!(result.as_str(), redirect);
}

#[test]
fn get_location_preserves_quoting_with_user_if_not_inheriting() {
    let redirect = "http://redirected.example.org/foo?response-content-disposition=attachment%3Bfilename%3D%22bar.tar.gz%22";
    let result = get_location(Some(redirect), &uri("http://a:b@original.example.org")).unwrap();
    assert_eq!(result, uri(redirect));
    assert_eq!(result.as_str(), redirect);
}
