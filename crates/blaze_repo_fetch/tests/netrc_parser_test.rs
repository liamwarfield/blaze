//! Port of `NetrcParserTest` and `NetrcCredentialsTest`.
// Java: src/test/java/com/google/devtools/build/lib/authandtls/NetrcParserTest.java

use std::collections::HashMap;

use blaze_repo_fetch::authandtls::netrc_parser::parse_and_close;
use blaze_repo_fetch::authandtls::{Credentials, Netrc, NetrcCredential, NetrcCredentials};
use blaze_repo_fetch::uri::JavaUri;

const FOO_MACHINE: &str = "foo.example.org";
const BAR_MACHINE: &str = "bar.example.org";

fn foo_credential() -> NetrcCredential {
    NetrcCredential::builder(FOO_MACHINE)
        .set_login("foouser")
        .set_password("foopass")
}

fn bar_credential() -> NetrcCredential {
    NetrcCredential::builder(BAR_MACHINE)
        .set_login("baruser")
        .set_password("barpass")
}

fn default_credential() -> NetrcCredential {
    NetrcCredential::builder("default")
        .set_login("defaultuser")
        .set_password("defaultpass")
}

fn parse(content: &str) -> Result<Netrc, blaze_repo_fetch::errors::IoException> {
    parse_and_close(content.as_bytes())
}

fn creds(entries: &[(&str, NetrcCredential)]) -> HashMap<String, NetrcCredential> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn machine_line(machine: &str, c: &NetrcCredential) -> String {
    format!(
        "machine {} login {} password {}",
        machine, c.login, c.password
    )
}

#[test]
fn parse_and_close_empty_content_return_empty() {
    let netrc = parse("").unwrap();
    assert_eq!(netrc.default_credential, None);
    assert!(netrc.credentials.is_empty());
}

#[test]
fn parse_and_close_empty_content_with_whitespaces_return_empty() {
    let netrc = parse("\t \n   \r\n  \n").unwrap();
    assert_eq!(netrc.default_credential, None);
    assert!(netrc.credentials.is_empty());
}

#[test]
fn parse_and_close_multiple_machines_return_matched() {
    let content = format!(
        "{}\n{}",
        machine_line(FOO_MACHINE, &foo_credential()),
        machine_line(BAR_MACHINE, &bar_credential())
    );
    let netrc = parse(&content).unwrap();
    assert_eq!(netrc.default_credential, None);
    assert_eq!(
        netrc.credentials,
        creds(&[
            (FOO_MACHINE, foo_credential()),
            (BAR_MACHINE, bar_credential())
        ])
    );
}

#[test]
fn parse_and_close_correct_but_badly_formatted_content_return_matched() {
    let content = format!(
        "machine {}\r\n   login {}\n\t\t\t password \t\t\n{}\n",
        FOO_MACHINE,
        foo_credential().login,
        foo_credential().password
    );
    let netrc = parse(&content).unwrap();
    assert_eq!(netrc.default_credential, None);
    assert_eq!(netrc.credentials, creds(&[(FOO_MACHINE, foo_credential())]));
}

#[test]
fn parse_and_close_macdef_only_return_empty() {
    let netrc = parse("macdef init\n\tcd /pub\n\tmget *\n\tquit").unwrap();
    assert_eq!(netrc.default_credential, None);
    assert!(netrc.credentials.is_empty());
}

#[test]
fn parse_and_close_mix_of_machines_and_macdef_skip_macdef_and_return_matched() {
    let content = format!(
        "{}\nmacdef init\n\tcd /pub\n\tmget *\n\tquit\n\n{}",
        machine_line(FOO_MACHINE, &foo_credential()),
        machine_line(BAR_MACHINE, &bar_credential())
    );
    let netrc = parse(&content).unwrap();
    assert_eq!(netrc.default_credential, None);
    assert_eq!(
        netrc.credentials,
        creds(&[
            (FOO_MACHINE, foo_credential()),
            (BAR_MACHINE, bar_credential())
        ])
    );
}

#[test]
fn parse_and_close_macdef_with_comments_in_between_skip_macdef_and_return_matched() {
    let content = format!(
        "{}\nmacdef init\n# this is comment\n\tcd /pub\n\tmget *\n\tquit\n# this is comment\n\n{}",
        machine_line(FOO_MACHINE, &foo_credential()),
        machine_line(BAR_MACHINE, &bar_credential())
    );
    let netrc = parse(&content).unwrap();
    assert_eq!(netrc.default_credential, None);
    assert_eq!(
        netrc.credentials,
        creds(&[
            (FOO_MACHINE, foo_credential()),
            (BAR_MACHINE, bar_credential())
        ])
    );
}

#[test]
fn parse_and_close_duplicated_machine_fields_override() {
    let content = format!(
        "machine {} login overridden_user login {} password overridden_pass password {}",
        FOO_MACHINE,
        foo_credential().login,
        foo_credential().password
    );
    let netrc = parse(&content).unwrap();
    assert_eq!(netrc.default_credential, None);
    assert_eq!(netrc.credentials, creds(&[(FOO_MACHINE, foo_credential())]));
}

#[test]
fn parse_and_close_machines_after_default_ignore() {
    let content = format!(
        "{}\ndefault login {} password {}\n{}",
        machine_line(FOO_MACHINE, &foo_credential()),
        default_credential().login,
        default_credential().password,
        machine_line(BAR_MACHINE, &bar_credential())
    );
    let netrc = parse(&content).unwrap();
    assert_eq!(netrc.default_credential, Some(default_credential()));
    assert_eq!(netrc.credentials, creds(&[(FOO_MACHINE, foo_credential())]));
}

#[test]
fn parse_and_close_bad_starting_content_fail() {
    assert!(parse("this is not netrc syntax").is_err());
}

#[test]
fn parse_and_close_bad_machine_fail() {
    assert!(parse("machine this is not netrc syntax").is_err());
}

#[test]
fn parse_and_close_bad_default_fail() {
    assert!(parse("default this is not netrc syntax").is_err());
}

#[test]
fn parse_and_close_comment_only_return_empty() {
    let netrc = parse("# this is comment").unwrap();
    assert_eq!(netrc.default_credential, None);
    assert!(netrc.credentials.is_empty());
}

#[test]
fn credential_should_not_leak_password() {
    assert!(!format!("{:?}", foo_credential()).contains(&foo_credential().password));
}

/// Blaze-only: `.netrc` is read as ISO-8859-1 (NetrcParser.java:80), so non-ASCII bytes become
/// one char each, and the `Authorization` header carries the internal string's bytes
/// (`StringUnsafe.getInternalStringBytes`, BasicHttpAuthenticationEncoder.java:39).
#[test]
fn parse_and_close_non_ascii_credentials_keep_their_bytes() {
    let netrc = parse_and_close(
        b"machine foo.example.org login l\xc3\xa9 password p\xf0\x9f\x90\xb7\xff\n".as_slice(),
    )
    .unwrap();
    let c = &netrc.credentials[FOO_MACHINE];
    assert_eq!(c.login, "l\u{c3}\u{a9}");
    assert_eq!(c.password, "p\u{f0}\u{9f}\u{90}\u{b7}\u{ff}");
    use base64::Engine as _;
    assert_eq!(
        blaze_repo_fetch::authandtls::basic_http_authentication_encode(&c.login, &c.password),
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(b"l\xc3\xa9:p\xf0\x9f\x90\xb7\xff")
        )
    );
}

// Java: src/test/java/com/google/devtools/build/lib/authandtls/NetrcCredentialsTest.java
mod netrc_credentials_test {
    use super::*;
    use blaze_repo_fetch::authandtls::{Headers, basic_http_authentication_encode};

    fn metadata(netrc: Netrc, url: &str) -> Headers {
        NetrcCredentials::new(netrc)
            .get_request_metadata(&JavaUri::create(url).unwrap())
            .unwrap()
    }

    fn assert_request_metadata(m: &Headers, username: &str, password: &str) {
        assert_eq!(m.keys().collect::<Vec<_>>(), vec!["Authorization"]);
        assert_eq!(
            m["Authorization"],
            vec![basic_http_authentication_encode(username, password)]
        );
    }

    #[test]
    fn get_request_metadata_empty_netrc_return_empty() {
        assert!(metadata(Netrc::create(None, HashMap::new()), "https://example.org").is_empty());
    }

    #[test]
    fn get_request_metadata_matched_machine_return_matched_one() {
        let netrc = Netrc::create(None, creds(&[(FOO_MACHINE, foo_credential())]));
        let m = metadata(netrc, &format!("https://{}", FOO_MACHINE));
        assert_request_metadata(&m, "foouser", "foopass");
    }

    #[test]
    fn get_request_metadata_not_matched_machine_return_empty() {
        let netrc = Netrc::create(None, creds(&[(FOO_MACHINE, foo_credential())]));
        assert!(metadata(netrc, &format!("https://{}", BAR_MACHINE)).is_empty());
    }

    #[test]
    fn get_request_metadata_not_matched_machine_return_default() {
        let netrc = Netrc::create(
            Some(default_credential()),
            creds(&[(FOO_MACHINE, foo_credential())]),
        );
        let m = metadata(netrc, &format!("https://{}", BAR_MACHINE));
        assert_request_metadata(&m, "defaultuser", "defaultpass");
    }

    #[test]
    fn get_request_metadata_empty_login() {
        let c = NetrcCredential::builder(FOO_MACHINE).set_password("foopass");
        let netrc = Netrc::create(None, creds(&[(FOO_MACHINE, c)]));
        let m = metadata(netrc, &format!("https://{}", FOO_MACHINE));
        assert_request_metadata(&m, "", "foopass");
    }

    #[test]
    fn get_request_metadata_empty_password() {
        let c = NetrcCredential::builder(FOO_MACHINE).set_login("foouser");
        let netrc = Netrc::create(None, creds(&[(FOO_MACHINE, c)]));
        let m = metadata(netrc, &format!("https://{}", FOO_MACHINE));
        assert_request_metadata(&m, "foouser", "");
    }

    #[test]
    fn get_request_metadata_empty_login_and_password() {
        let netrc = Netrc::create(
            None,
            creds(&[(FOO_MACHINE, NetrcCredential::builder(FOO_MACHINE))]),
        );
        let m = metadata(netrc, &format!("https://{}", FOO_MACHINE));
        assert_request_metadata(&m, "", "");
    }
}
