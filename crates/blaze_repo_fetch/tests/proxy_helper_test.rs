//! Port of `ProxyHelperTest`.

use std::collections::HashMap;

use base64::Engine as _;
use blaze_repo_fetch::downloader::proxy_helper::{
    ProxyHelper, ProxyInfo, ProxyType, create_proxy, create_proxy_info,
};
use blaze_repo_fetch::uri::JavaUri;
use regex::Regex;

fn helper(pairs: &[(&str, &str)]) -> ProxyHelper {
    ProxyHelper::new(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<HashMap<_, _>>(),
    )
}

fn create_if_needed(h: &ProxyHelper, url: &str) -> ProxyInfo {
    h.create_proxy_if_needed(&JavaUri::create(url).unwrap())
        .unwrap()
}

fn contains_match(s: &str, re: &str) {
    assert!(Regex::new(re).unwrap().is_match(s), "{} !~ {}", s, re);
}

fn is_no_proxy(p: &ProxyInfo) -> bool {
    p.proxy_type == ProxyType::DIRECT
}

fn decoded_auth(p: &ProxyInfo) -> String {
    let header = p.get_proxy_authorization_header().unwrap();
    let encoded = header.strip_prefix("Basic ").unwrap();
    String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn test_create_if_needed_http_lower_case() {
    let p = create_if_needed(
        &helper(&[("http_proxy", "http://my.example.com")]),
        "http://www.something.com",
    );
    contains_match(
        &p.proxy_to_string(),
        r"my\.example\.com(/<unresolved>)?:80$",
    );
}

#[test]
fn test_create_if_needed_http_upper_case() {
    let p = create_if_needed(
        &helper(&[("HTTP_PROXY", "http://my.example.com")]),
        "http://www.something.com",
    );
    contains_match(
        &p.proxy_to_string(),
        r"my\.example\.com(/<unresolved>)?:80$",
    );
}

#[test]
fn test_create_if_needed_https_lower_case() {
    let p = create_if_needed(
        &helper(&[("https_proxy", "https://my.example.com")]),
        "https://www.something.com",
    );
    contains_match(
        &p.proxy_to_string(),
        r"my\.example\.com(/<unresolved>)?:443$",
    );
}

#[test]
fn test_create_if_needed_https_upper_case() {
    let p = create_if_needed(
        &helper(&[("HTTPS_PROXY", "https://my.example.com")]),
        "https://www.something.com",
    );
    contains_match(
        &p.proxy_to_string(),
        r"my\.example\.com(/<unresolved>)?:443$",
    );
}

#[test]
fn test_create_if_needed_no_proxy_lower_case() {
    let h = helper(&[
        ("no_proxy", "something.com,example.com,localhost"),
        ("HTTPS_PROXY", "https://my.example.com"),
    ]);
    assert!(is_no_proxy(&create_if_needed(
        &h,
        "https://www.example.com"
    )));
}

#[test]
fn test_create_if_needed_no_proxy_upper_case() {
    let h = helper(&[
        ("NO_PROXY", "something.com,example.com,localhost"),
        ("HTTPS_PROXY", "https://my.example.com"),
    ]);
    assert!(is_no_proxy(&create_if_needed(
        &h,
        "https://www.example.com"
    )));
}

#[test]
fn test_create_if_needed_multiple_no_proxy_lower_case() {
    let h = helper(&[
        ("no_proxy", "something.com,example.com,localhost"),
        ("HTTPS_PROXY", "https://my.example.com"),
    ]);
    assert!(is_no_proxy(&create_if_needed(
        &h,
        "https://www.example.com"
    )));
}

#[test]
fn test_create_if_needed_multiple_no_proxy_upper_case() {
    let h = helper(&[
        ("NO_PROXY", "something.com,example.com,localhost"),
        ("HTTPS_PROXY", "https://my.example.com"),
    ]);
    assert!(is_no_proxy(&create_if_needed(
        &h,
        "https://www.example.com"
    )));
}

#[test]
fn test_create_if_needed_multiple_no_proxy_spaces() {
    let h = helper(&[
        ("no_proxy", "something.com ,   example.com, localhost"),
        ("HTTPS_PROXY", "https://my.example.com"),
    ]);
    assert!(is_no_proxy(&create_if_needed(
        &h,
        "https://www.something.com"
    )));
    assert!(is_no_proxy(&create_if_needed(
        &h,
        "https://www.example.com"
    )));
    assert!(is_no_proxy(&create_if_needed(&h, "https://localhost")));
}

#[test]
fn test_create_if_needed_no_proxy_no_match_substring() {
    let h = helper(&[
        ("NO_PROXY", "something.com,example.com,localhost"),
        ("HTTPS_PROXY", "https://my.example.com"),
    ]);
    let p = create_if_needed(&h, "https://www.not-example.com");
    contains_match(
        &p.proxy_to_string(),
        r"my\.example\.com(/<unresolved>)?:443$",
    );
}

#[test]
fn test_create_if_needed_no_proxy_match_subdomain_in_no_proxy() {
    let h = helper(&[
        ("NO_PROXY", ".something.com,example.com,localhost"),
        ("HTTPS_PROXY", "https://my.example.com"),
    ]);
    assert!(is_no_proxy(&create_if_needed(
        &h,
        "https://www.my.something.com"
    )));
}

#[test]
fn test_create_if_needed_no_proxy_match_subdomain_in_url() {
    let h = helper(&[
        ("NO_PROXY", "something.com,example.com,localhost"),
        ("HTTPS_PROXY", "https://my.example.com"),
    ]);
    assert!(is_no_proxy(&create_if_needed(
        &h,
        "https://www.my.subdomain.something.com"
    )));
}

#[test]
fn test_no_proxy() {
    assert!(is_no_proxy(&create_proxy(None).unwrap()));
    assert!(is_no_proxy(&create_proxy(Some("")).unwrap()));
    assert!(is_no_proxy(&create_if_needed(
        &helper(&[]),
        "https://www.something.com"
    )));
}

#[test]
fn test_proxy_default_port() {
    let p = create_proxy(Some("http://my.example.com")).unwrap();
    assert_eq!(p.proxy_type, ProxyType::HTTP);
    assert!(p.proxy_to_string().ends_with(":80"));
    let p = create_proxy(Some("https://my.example.com")).unwrap();
    assert!(p.proxy_to_string().ends_with(":443"));
}

#[test]
fn test_proxy_explicit_port() {
    assert!(
        create_proxy(Some("http://my.example.com:12345"))
            .unwrap()
            .proxy_to_string()
            .ends_with(":12345")
    );
    assert!(
        create_proxy(Some("https://my.example.com:12345"))
            .unwrap()
            .proxy_to_string()
            .ends_with(":12345")
    );
}

#[test]
fn test_proxy_no_protocol() {
    assert!(
        create_proxy(Some("my.example.com"))
            .unwrap()
            .proxy_to_string()
            .ends_with(":80")
    );
}

#[test]
fn test_proxy_no_protocol_with_port() {
    assert!(
        create_proxy(Some("my.example.com:12345"))
            .unwrap()
            .proxy_to_string()
            .ends_with(":12345")
    );
}

#[test]
fn test_proxy_port_parsing_error() {
    let e = create_proxy(Some("http://my.example.com:foo")).unwrap_err();
    assert!(
        e.get_message()
            .contains("Proxy address http://my.example.com:foo is not a valid URL")
    );
}

#[test]
fn test_proxy_auth() {
    let p = create_proxy(Some("http://foo:barbaz@my.example.com")).unwrap();
    assert_eq!(p.proxy_type, ProxyType::HTTP);
    assert!(p.proxy_to_string().ends_with(":80"));
    let p = create_proxy(Some("https://biz:bat@my.example.com")).unwrap();
    assert!(p.proxy_to_string().ends_with(":443"));
}

#[test]
fn test_encoded_proxy_auth() {
    let p = create_proxy(Some("http://foo:b%40rb%40z@my.example.com")).unwrap();
    assert_eq!(p.proxy_type, ProxyType::HTTP);
    assert!(p.proxy_to_string().ends_with(":80"));
}

#[test]
fn test_invalid_auth() {
    let e = create_proxy(Some("http://foo@my.example.com")).unwrap_err();
    assert!(e.get_message().contains("No password given for proxy"));
}

#[test]
fn test_no_proxy_auth() {
    let p = create_proxy(Some("http://localhost:3128/")).unwrap();
    assert_eq!(p.proxy_type, ProxyType::HTTP);
    assert!(p.proxy_to_string().ends_with(":3128"));
}

#[test]
fn test_trailing_slash() {
    let p = create_proxy(Some("http://foo:bar@example.com:8000/")).unwrap();
    assert_eq!(p.proxy_type, ProxyType::HTTP);
    assert!(p.proxy_to_string().ends_with(":8000"));
}

#[test]
fn test_proxy_info_with_credentials() {
    let p = create_proxy(Some("http://myuser:mypass@proxy.example.com:8080")).unwrap();
    assert!(p.has_credentials());
    assert!(
        p.get_proxy_authorization_header()
            .unwrap()
            .starts_with("Basic ")
    );
    assert_eq!(decoded_auth(&p), "myuser:mypass");
}

#[test]
fn test_proxy_info_with_url_encoded_credentials() {
    let p = create_proxy(Some("http://user:p%40ss%3Aword@proxy.example.com:8080")).unwrap();
    assert!(p.has_credentials());
    assert_eq!(decoded_auth(&p), "user:p@ss:word");
}

#[test]
fn test_proxy_info_with_url_encoded_username() {
    let p = create_proxy(Some("http://user%40domain:password@proxy.example.com:8080")).unwrap();
    assert!(p.has_credentials());
    assert_eq!(decoded_auth(&p), "user@domain:password");
}

#[test]
fn test_proxy_info_with_unicode_credentials() {
    let p = create_proxy(Some(
        "http://%E7%94%A8%E6%88%B7:contrase%C3%B1a@proxy.example.com:8080",
    ))
    .unwrap();
    assert!(p.has_credentials());
    assert_eq!(decoded_auth(&p), "用户:contraseña");
}

#[test]
fn test_proxy_info_without_credentials() {
    let p = create_proxy(Some("http://proxy.example.com:8080")).unwrap();
    assert!(!p.has_credentials());
    assert!(p.get_proxy_authorization_header().is_none());
}

#[test]
fn test_proxy_info_no_proxy_has_no_credentials() {
    let p = ProxyInfo::no_proxy();
    assert!(!p.has_credentials());
    assert!(p.get_proxy_authorization_header().is_none());
    assert!(is_no_proxy(&p));
}

#[test]
fn test_proxy_info_from_system_properties() {
    let p = create_proxy_info(
        Some("http://proxy.example.com:8080"),
        Some("sysuser"),
        Some("syspass"),
    )
    .unwrap();
    assert!(p.has_credentials());
    assert_eq!(decoded_auth(&p), "sysuser:syspass");
}

#[test]
fn test_proxy_info_url_credentials_take_precedence() {
    let p = create_proxy_info(
        Some("http://urluser:urlpass@proxy.example.com:8080"),
        Some("sysuser"),
        Some("syspass"),
    )
    .unwrap();
    assert!(p.has_credentials());
    assert_eq!(decoded_auth(&p), "urluser:urlpass");
}

#[test]
fn test_proxy_info_system_properties_only_user_ignored() {
    let p =
        create_proxy_info(Some("http://proxy.example.com:8080"), Some("sysuser"), None).unwrap();
    assert!(!p.has_credentials());
}

#[test]
fn test_proxy_info_system_properties_only_password_ignored() {
    let p =
        create_proxy_info(Some("http://proxy.example.com:8080"), None, Some("syspass")).unwrap();
    assert!(!p.has_credentials());
}

#[test]
#[ignore = "needs JVM system property http.nonProxyHosts, which Blaze does not have"]
fn test_non_proxy_hosts_exact_match() {}

#[test]
#[ignore = "needs JVM system property http.nonProxyHosts, which Blaze does not have"]
fn test_non_proxy_hosts_wildcard_prefix() {}

#[test]
#[ignore = "needs JVM system property http.nonProxyHosts, which Blaze does not have"]
fn test_non_proxy_hosts_wildcard_suffix() {}

#[test]
fn test_socks5_proxy_default_port() {
    let p = create_proxy(Some("socks5://my.example.com")).unwrap();
    assert_eq!(p.proxy_type, ProxyType::SOCKS);
    assert!(p.proxy_to_string().ends_with(":1080"));
}

#[test]
fn test_socks5_proxy_explicit_port() {
    let p = create_proxy(Some("socks5://my.example.com:5000")).unwrap();
    assert_eq!(p.proxy_type, ProxyType::SOCKS);
    assert!(p.proxy_to_string().ends_with(":5000"));
}

#[test]
fn test_socks4_proxy_default_port() {
    let p = create_proxy(Some("socks4://my.example.com")).unwrap();
    assert_eq!(p.proxy_type, ProxyType::SOCKS);
    assert!(p.proxy_to_string().ends_with(":1080"));
}

#[test]
fn test_socks_proxy_default_port() {
    let p = create_proxy(Some("socks://my.example.com")).unwrap();
    assert_eq!(p.proxy_type, ProxyType::SOCKS);
    assert!(p.proxy_to_string().ends_with(":1080"));
}

#[test]
fn test_socks5_proxy_with_auth() {
    let p = create_proxy(Some("socks5://user:pass@my.example.com:1080")).unwrap();
    assert_eq!(p.proxy_type, ProxyType::SOCKS);
    assert!(p.proxy_to_string().ends_with(":1080"));
    assert!(p.has_credentials());
    assert_eq!(decoded_auth(&p), "user:pass");
}

#[test]
fn test_create_if_needed_socks5_proxy() {
    let p = create_if_needed(
        &helper(&[("HTTPS_PROXY", "socks5://localhost:5000")]),
        "https://www.something.com",
    );
    assert_eq!(p.proxy_type, ProxyType::SOCKS);
    assert!(p.proxy_to_string().contains("localhost"));
    assert!(p.proxy_to_string().ends_with(":5000"));
}
