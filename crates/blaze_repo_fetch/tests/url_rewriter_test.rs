//! Port of `UrlRewriterTest`.
// Java: src/test/java/com/google/devtools/build/lib/bazel/repository/downloader/UrlRewriterTest.java

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
use blaze_events::location::Location;
use blaze_repo_fetch::authandtls::{Credentials, Headers, basic_http_authentication_encode};
use blaze_repo_fetch::downloader::url_rewriter::{
    RewrittenUrl, UrlRewriter, UrlRewriterConfig, UrlRewriterParseException,
};
use blaze_repo_fetch::uri::JavaUri;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};

fn uri(s: &str) -> JavaUri {
    JavaUri::create(s).unwrap()
}

fn test_url_rewriter(path: &str, config: &str) -> Result<UrlRewriter, UrlRewriterParseException> {
    UrlRewriter::new(&[path.to_string()], &[config.to_string()])
}

fn amend_urls(munger: &UrlRewriter, urls: &[&str]) -> Vec<JavaUri> {
    let urls: Vec<JavaUri> = urls.iter().map(|u| uri(u)).collect();
    munger.amend(&urls).into_iter().map(|u| u.url).collect()
}

fn fs() -> FileSystemRef {
    UnixFileSystem::create(DigestHashFunction::Sha256)
}

fn path(fs: &FileSystemRef, p: &std::path::Path) -> Path {
    Path::create_from_str(p.to_str().unwrap(), fs)
}

#[test]
fn by_default_the_url_rewriter_does_nothing() {
    let munger = test_url_rewriter("/dev/null", "").unwrap();
    assert_eq!(
        amend_urls(&munger, &["http://example.com"]),
        vec![uri("http://example.com")]
    );
}

#[test]
#[should_panic(expected = "filePath and readers size must be equal")]
fn constructor_must_have_the_same_number_of_file_paths_and_readers() {
    let _ = UrlRewriter::new(
        &["/dev/null".to_string(), "/dev/null".to_string()],
        &[String::new()],
    );
}

#[test]
fn should_be_able_to_block_particular_hosts_regardless_of_scheme() {
    let munger = test_url_rewriter("/dev/null", "block example.com").unwrap();
    assert_eq!(
        amend_urls(
            &munger,
            &[
                "http://example.com",
                "https://example.com",
                "http://localhost"
            ]
        ),
        vec![uri("http://localhost")]
    );
}

#[test]
fn should_allow_a_url_to_be_rewritten() {
    let munger = test_url_rewriter(
        "/dev/null",
        "rewrite example.com/foo/(.*) mycorp.com/$1/foo",
    )
    .unwrap();
    assert_eq!(
        amend_urls(&munger, &["https://example.com/foo/bar"]),
        vec![uri("https://mycorp.com/bar/foo")]
    );
}

#[test]
fn rewrites_can_expand_to_more_than_one_url() {
    let config = "rewrite example.com/foo/(.*) mycorp.com/$1/somewhere\n\
                  rewrite example.com/foo/(.*) mycorp.com/$1/elsewhere";
    let munger = test_url_rewriter("/dev/null", config).unwrap();
    let amended = amend_urls(&munger, &["https://example.com/foo/bar"]);
    assert!(amended.contains(&uri("https://mycorp.com/bar/somewhere")));
    assert!(amended.contains(&uri("https://mycorp.com/bar/elsewhere")));
}

#[test]
fn rewrites_can_expand_to_more_than_one_url_with_multiple_configs() {
    let munger = UrlRewriter::new(
        &["/dev/null".to_string(), "/dev/null".to_string()],
        &[
            "rewrite example.com/foo/(.*) mycorp.com/$1/somewhere\n".to_string(),
            "rewrite example.com/foo/(.*) mycorp.com/$1/elsewhere\n".to_string(),
        ],
    )
    .unwrap();
    let amended = amend_urls(&munger, &["https://example.com/foo/bar"]);
    assert!(amended.contains(&uri("https://mycorp.com/bar/somewhere")));
    assert!(amended.contains(&uri("https://mycorp.com/bar/elsewhere")));
}

#[test]
fn should_block_all_urls_other_than_specific_ones() {
    let munger = test_url_rewriter("/dev/null", "block *\nallow example.com").unwrap();
    assert_eq!(
        amend_urls(
            &munger,
            &[
                "https://foo.com",
                "https://example.com/foo/bar",
                "https://subdomain.example.com/qux"
            ]
        ),
        vec![
            uri("https://example.com/foo/bar"),
            uri("https://subdomain.example.com/qux")
        ]
    );
}

#[test]
fn comments_are_preceded_by_the_hash_character() {
    let config = "# Block everything\nblock *\n# But allow example.com\nallow example.com";
    let munger = test_url_rewriter("/dev/null", config).unwrap();
    assert_eq!(
        amend_urls(&munger, &["https://foo.com", "https://example.com"]),
        vec![uri("https://example.com")]
    );
}

#[test]
fn allow_list_applies_to_subdomains_too() {
    let munger = test_url_rewriter("/dev/null", "block *\nallow example.com").unwrap();
    assert_eq!(
        amend_urls(&munger, &["https://subdomain.example.com"]),
        vec![uri("https://subdomain.example.com")]
    );
}

#[test]
fn block_list_applies_to_subdomains_too() {
    let munger = test_url_rewriter("/dev/null", "block example.com").unwrap();
    assert!(amend_urls(&munger, &["https://subdomain.example.com"]).is_empty());
}

#[test]
fn empty_lines_are_fine() {
    let munger = test_url_rewriter("/dev/null", "\n   \nblock *\n\t  \nallow example.com").unwrap();
    assert_eq!(
        amend_urls(&munger, &["https://subdomain.example.com"]),
        vec![uri("https://subdomain.example.com")]
    );
}

#[test]
fn rewriting_urls_is_applied_before_blocking() {
    let munger = test_url_rewriter(
        "/dev/null",
        "block bad.com\nrewrite bad.com/foo/(.*) mycorp.com/$1",
    )
    .unwrap();
    assert_eq!(
        amend_urls(&munger, &["https://www.bad.com", "https://bad.com/foo/bar"]),
        vec![uri("https://mycorp.com/bar")]
    );
}

#[test]
fn rewriting_urls_is_applied_before_allowing() {
    let munger = test_url_rewriter(
        "/dev/null",
        "block *\nallow mycorp.com\nrewrite bad.com/foo/(.*) mycorp.com/$1",
    )
    .unwrap();
    assert_eq!(
        amend_urls(&munger, &["https://www.bad.com", "https://bad.com/foo/bar"]),
        vec![uri("https://mycorp.com/bar")]
    );
}

#[test]
fn parse_error() {
    let config = "#comment\nhello";
    assert!(test_url_rewriter("/some/file", config).is_err());
    let e = UrlRewriterConfig::new("/some/file", config).unwrap_err();
    assert_eq!(
        e.location,
        Some(Location::from_file_line_column("/some/file", 2, 0))
    );
    assert!(e.message.contains("Unable to parse: hello"));
}

#[test]
fn no_all_blocked_message() {
    let munger = UrlRewriterConfig::new("/some/file", "").unwrap();
    assert_eq!(munger.get_all_blocked_message(), None);
}

#[test]
fn single_all_blocked_message() {
    let config = "all_blocked_message I'm sorry Dave, I'm afraid I can't do that.\nallow *\n";
    let munger = UrlRewriterConfig::new("/some/file", config).unwrap();
    assert_eq!(
        munger.get_all_blocked_message(),
        Some("I'm sorry Dave, I'm afraid I can't do that.")
    );
}

#[test]
fn multiple_all_blocked_message() {
    let config = "all_blocked_message one\nblock *\nall_blocked_message two\n";
    let e = UrlRewriterConfig::new("/some/file", config).unwrap_err();
    assert_eq!(
        e.location,
        Some(Location::from_file_line_column("/some/file", 3, 0))
    );
}

#[test]
fn rewriting_urls_allows_protocol_rewrite() {
    let config = "block *\n\
                  allow mycorp.com\n\
                  allow othercorp.com\n\
                  rewrite bad.com/foo/(.*) http://mycorp.com/$1\n\
                  rewrite bad.com/bar/(.*) https://othercorp.com/bar/$1\n";
    let munger = test_url_rewriter("/dev/null", config).unwrap();
    assert_eq!(
        amend_urls(
            &munger,
            &[
                "https://www.bad.com",
                "https://bad.com/foo/bar",
                "http://bad.com/bar/xyz"
            ]
        ),
        vec![
            uri("http://mycorp.com/bar"),
            uri("https://othercorp.com/bar/xyz")
        ]
    );
}

fn basic(creds: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(creds.as_bytes())
    )
}

fn auth(token: String) -> Headers {
    let mut h = Headers::new();
    h.insert("Authorization".to_string(), vec![token]);
    h
}

#[test]
fn rewriting_urls_with_auth_headers() {
    let tmp = tempfile::tempdir().unwrap();
    let netrc = parse_netrc(
        &tmp,
        "machine mycorp.com login netrc_user_0 password netrc_pw_0\n\
         machine myothercorp.com login netrc_user_1 password netrc_pw_1\n\
         machine no-override.com login netrc_user_2 password netrc_pw_2\n",
    )
    .unwrap()
    .unwrap();
    let creds = "user:password";
    let config = format!(
        "rewrite my.example.com/foo/(.*) {}@mycorp.com/foo/$1\n\
         rewrite my.example.com/from_netrc/(.*) mycorp.com/from_netrc/$1\n\
         rewrite my.example.com/from_other_netrc_entry/(.*) myothercorp.com/from_netrc/$1\n\
         rewrite my.example.com/no_creds/(.*) myopencorp.com/no_creds/$1\n",
        creds
    );
    let munger = test_url_rewriter("/dev/null", &config).unwrap();
    let amended = munger.amend(&[
        uri("https://my.example.com/foo/bar"),
        uri("https://my.example.com/from_netrc/bar"),
        uri("https://my.example.com/from_other_netrc_entry/bar"),
        uri("https://my.example.com/no_creds/bar"),
        uri("https://should-not-be-overridden.com/"),
    ]);
    let updated = munger.update_auth_headers(&amended, &HashMap::new(), Some(netrc.as_ref()));
    let mut expected: HashMap<JavaUri, Headers> = HashMap::new();
    expected.insert(
        uri("https://user:password@mycorp.com/foo/bar"),
        auth(basic(creds)),
    );
    expected.insert(
        uri("https://mycorp.com/from_netrc/bar"),
        auth(basic("netrc_user_0:netrc_pw_0")),
    );
    expected.insert(
        uri("https://myothercorp.com/from_netrc/bar"),
        auth(basic("netrc_user_1:netrc_pw_1")),
    );
    assert_eq!(updated, expected);
    assert_eq!(
        amended,
        vec![
            RewrittenUrl {
                url: uri("https://user:password@mycorp.com/foo/bar"),
                rewritten: true
            },
            RewrittenUrl {
                url: uri("https://mycorp.com/from_netrc/bar"),
                rewritten: true
            },
            RewrittenUrl {
                url: uri("https://myothercorp.com/from_netrc/bar"),
                rewritten: true
            },
            RewrittenUrl {
                url: uri("https://myopencorp.com/no_creds/bar"),
                rewritten: true
            },
            RewrittenUrl {
                url: uri("https://should-not-be-overridden.com/"),
                rewritten: false
            },
        ]
    );
}

#[test]
fn test_netrc_empty_env_should_ignore() {
    let fs = fs();
    let working_dir = Path::create_from_str("/workdir", &fs);
    let credentials =
        UrlRewriter::new_credentials_from_netrc(&HashMap::new(), Some(&working_dir)).unwrap();
    assert!(credentials.is_none());
}

#[test]
fn test_netrc_netrc_not_exist_should_ignore() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home/foo");
    let env: HashMap<String, String> = [
        ("HOME".to_string(), home.to_str().unwrap().to_string()),
        (
            "USERPROFILE".to_string(),
            home.to_str().unwrap().to_string(),
        ),
    ]
    .into();
    let fs = fs();
    let working_dir = path(&fs, &tmp.path().join("workdir"));
    let credentials = UrlRewriter::new_credentials_from_netrc(&env, Some(&working_dir)).unwrap();
    assert!(credentials.is_none());
}

fn assert_request_metadata(m: &Headers, username: &str, password: &str) {
    assert_eq!(m.keys().collect::<Vec<_>>(), vec!["Authorization"]);
    assert_eq!(
        m["Authorization"],
        vec![basic_http_authentication_encode(username, password)]
    );
}

#[test]
fn test_netrc_relative_netrc_should_use() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path().join("workdir");
    std::fs::create_dir_all(workdir.join("foo")).unwrap();
    std::fs::write(
        workdir.join("foo/.netrc"),
        "machine foo.example.org login foouser password foopass\n",
    )
    .unwrap();
    let env: HashMap<String, String> = [("NETRC".to_string(), "./foo/.netrc".to_string())].into();
    let fs = fs();
    let working_dir = path(&fs, &workdir);
    let credentials = UrlRewriter::new_credentials_from_netrc(&env, Some(&working_dir))
        .unwrap()
        .unwrap();
    assert_request_metadata(
        &credentials
            .get_request_metadata(&uri("https://foo.example.org"))
            .unwrap(),
        "foouser",
        "foopass",
    );
}

#[test]
fn test_netrc_relative_netrc_should_ignore_when_not_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let env: HashMap<String, String> = [("NETRC".to_string(), "./foo/.netrc".to_string())].into();
    let fs = fs();
    let working_dir = path(&fs, &tmp.path().join("workdir"));
    let credentials = UrlRewriter::new_credentials_from_netrc(&env, Some(&working_dir)).unwrap();
    assert!(credentials.is_none());
}

#[test]
fn test_netrc_netrc_exist_should_use() {
    let tmp = tempfile::tempdir().unwrap();
    let credentials = parse_netrc(
        &tmp,
        "machine foo.example.org login foouser password foopass",
    )
    .unwrap()
    .unwrap();
    assert_request_metadata(
        &credentials
            .get_request_metadata(&uri("https://foo.example.org"))
            .unwrap(),
        "foouser",
        "foopass",
    );
}

#[test]
fn test_netrc_netrc_exist_cant_parse() {
    let tmp = tempfile::tempdir().unwrap();
    let e = match parse_netrc(&tmp, "mach foo.example.org log foouser password foopass") {
        Err(e) => e,
        Ok(_) => panic!("expected UrlRewriterParseException"),
    };
    // PORT: the Java test's /home/foo lives under the temporary directory here.
    let netrc = tmp.path().join("home/foo/.netrc");
    assert_eq!(
        e.location,
        Some(Location::from_file_line_column(
            netrc.to_str().unwrap(),
            0,
            0
        ))
    );
}

#[test]
fn rewrite_rule_producing_null_host_uri_does_not_crash() {
    let config = "block *\nrewrite (example.com)/(.*) @UNSUBSTITUTED@/$1/$2";
    let rewriter = test_url_rewriter("/dev/null", config).unwrap();
    assert!(amend_urls(&rewriter, &["https://example.com/foo/bar"]).is_empty());
}

#[test]
fn allow_list_does_not_crash_on_null_host_uri() {
    let config = "allow example.com\nblock *\nrewrite (example.com)/(.*) @PLACEHOLDER@/$1/$2";
    let rewriter = test_url_rewriter("/dev/null", config).unwrap();
    assert!(amend_urls(&rewriter, &["https://example.com/foo"]).is_empty());
}

fn parse_netrc(
    tmp: &tempfile::TempDir,
    content: &str,
) -> Result<Option<Arc<dyn Credentials>>, UrlRewriterParseException> {
    let home = tmp.path().join("home/foo");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join(".netrc"), format!("{}\n", content)).unwrap();
    let env: HashMap<String, String> = [
        ("HOME".to_string(), home.to_str().unwrap().to_string()),
        (
            "USERPROFILE".to_string(),
            home.to_str().unwrap().to_string(),
        ),
    ]
    .into();
    let fs = fs();
    let working_dir = path(&fs, &tmp.path().join("workdir"));
    UrlRewriter::new_credentials_from_netrc(&env, Some(&working_dir))
}

// Byte-strings review 2026-11-20 S1-5. Java UrlRewriter.java:125 reads the config as UTF-8
// (Unicode) and :178 encodes the userInfo with `getBytes(ISO_8859_1)`: `中` (U+4E2D) becomes `?`,
// `é` (U+00E9) the byte E9.
#[test]
fn rewritten_url_user_info_is_encoded_as_latin1() {
    let munger = test_url_rewriter(
        "/dev/null",
        "rewrite a.example.com/(.*) 中:pw@mirror.example.com/$1\n\
         rewrite b.example.com/(.*) é:pw@mirror.example.com/$1\n",
    )
    .unwrap();
    let amended = munger.amend(&[
        uri("https://a.example.com/x"),
        uri("https://b.example.com/x"),
    ]);
    let updated = munger.update_auth_headers(&amended, &HashMap::new(), None);
    assert_eq!(
        updated.get(&amended[0].url),
        Some(&auth("Basic Pzpwdw==".to_string()))
    );
    assert_eq!(
        updated.get(&amended[1].url),
        Some(&auth("Basic 6Tpwdw==".to_string()))
    );
}
