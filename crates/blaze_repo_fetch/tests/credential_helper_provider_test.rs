//! Port of `CredentialHelperProviderTest`.
// Java: src/test/java/com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperProviderTest.java

use blaze_repo_fetch::authandtls::credentialhelper::CredentialHelperProvider;
use blaze_repo_fetch::authandtls::credentialhelper::credential_helper_provider::parent_domain;
use blaze_repo_fetch::uri::JavaUri;
use blaze_vfs::unix_file_system::UnixFileSystem;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path};

const DEFAULT_HELPER_PATH: &str = "/path/to/default/helper";
const EXAMPLE_COM_HELPER_PATH: &str = "/path/to/example/com/helper";
const EXAMPLE_COM_WILDCARD_HELPER_PATH: &str = "/path/to/example/com/wildcard/helper";
const SUB_EXAMPLE_COM_WILDCARD_HELPER_PATH: &str = "/path/to/sub/example/com/wildcard/helper";

/// PORT: Java creates the helpers as empty executable files in an `InMemoryFileSystem`; the
/// provider never reads them, so these are paths only.
fn file_system() -> FileSystemRef {
    UnixFileSystem::create(DigestHashFunction::Sha256)
}

fn path(fs: &FileSystemRef, p: &str) -> Path {
    Path::create_from_str(p, fs)
}

fn helper_path(provider: &CredentialHelperProvider, uri: &str) -> Option<Path> {
    provider
        .find_credential_helper(&JavaUri::create(uri).unwrap())
        .map(|h| h.get_path().clone())
}

#[test]
fn no_helpers_configured() {
    let provider = CredentialHelperProvider::builder().build();

    for uri in [
        "http://example.com/foo",
        "https://example.com/foo",
        "grpc://example.com/foo",
        "grpcs://example.com/foo",
        "custom://example.com/foo",
        "https://subdomain.example.com/bar",
        "https://other-domain.com",
    ] {
        assert_eq!(helper_path(&provider, uri), None, "{uri}");
    }
}

#[test]
fn uri_without_host_component() {
    let fs = file_system();
    let helper = path(&fs, EXAMPLE_COM_HELPER_PATH);
    let provider = CredentialHelperProvider::builder()
        .add("example.com", helper)
        .unwrap()
        .build();

    assert_eq!(helper_path(&provider, "unix:///path/to/socket"), None);
}

#[test]
fn only_default_helper() {
    let fs = file_system();
    let helper = path(&fs, DEFAULT_HELPER_PATH);
    let provider = CredentialHelperProvider::builder()
        .add_default(helper.clone())
        .unwrap()
        .build();

    for uri in [
        "http://example.com/foo",
        "https://example.com/foo",
        "grpc://example.com/foo",
        "grpcs://example.com/foo",
        "unix:///tmp/grpc.sock",
        "custom://example.com/foo",
        "https://subdomain.example.com/bar",
        "https://other-domain.com",
    ] {
        assert_eq!(helper_path(&provider, uri), Some(helper.clone()), "{uri}");
    }
}

#[test]
fn with_host_helpers_and_default_fallback() {
    let fs = file_system();
    let default_helper = path(&fs, DEFAULT_HELPER_PATH);
    let example_com_helper = path(&fs, EXAMPLE_COM_HELPER_PATH);
    let mut builder = CredentialHelperProvider::builder();
    builder.add_default(default_helper.clone()).unwrap();
    builder
        .add("example.com", example_com_helper.clone())
        .unwrap();
    let provider = builder.build();

    for uri in [
        "http://example.com/foo",
        "https://example.com/foo",
        "grpc://example.com/foo",
        "grpcs://example.com/foo",
        "custom://example.com/foo",
    ] {
        assert_eq!(
            helper_path(&provider, uri),
            Some(example_com_helper.clone()),
            "{uri}"
        );
    }

    for uri in [
        "https://subdomain.example.com/bar",
        "https://other-domain.com",
        "unix:///tmp/grpc.sock",
    ] {
        assert_eq!(
            helper_path(&provider, uri),
            Some(default_helper.clone()),
            "{uri}"
        );
    }
}

#[test]
fn wildcard_matching() {
    let fs = file_system();
    let default_helper = path(&fs, DEFAULT_HELPER_PATH);
    let example_com_wildcard_helper = path(&fs, EXAMPLE_COM_WILDCARD_HELPER_PATH);
    let mut builder = CredentialHelperProvider::builder();
    builder.add_default(default_helper.clone()).unwrap();
    builder
        .add("*.example.com", example_com_wildcard_helper.clone())
        .unwrap();
    let provider = builder.build();

    for uri in [
        "http://example.com/foo",
        "https://example.com/foo",
        "grpc://example.com/foo",
        "grpcs://example.com/foo",
        "custom://example.com/foo",
        "https://subdomain.example.com/bar",
        "https://subdomain2.example.com/bar",
        "https://sub.subdomain.example.com/bar",
        "https://subdomain.example.com/bar",
    ] {
        assert_eq!(
            helper_path(&provider, uri),
            Some(example_com_wildcard_helper.clone()),
            "{uri}"
        );
    }

    assert_eq!(
        helper_path(&provider, "https://other-domain.com"),
        Some(default_helper.clone())
    );

    assert_eq!(
        helper_path(&provider, "unix:///tmp/grpc.sock"),
        Some(default_helper)
    );
}

#[test]
fn prefer_exact_match_over_wildcard_matching() {
    let fs = file_system();
    let default_helper = path(&fs, DEFAULT_HELPER_PATH);
    let example_com_helper = path(&fs, EXAMPLE_COM_HELPER_PATH);
    let example_com_wildcard_helper = path(&fs, EXAMPLE_COM_WILDCARD_HELPER_PATH);
    let mut builder = CredentialHelperProvider::builder();
    builder.add_default(default_helper.clone()).unwrap();
    builder
        .add("example.com", example_com_helper.clone())
        .unwrap();
    builder
        .add("*.example.com", example_com_wildcard_helper.clone())
        .unwrap();
    let provider = builder.build();

    for uri in [
        "http://example.com/foo",
        "https://example.com/foo",
        "grpc://example.com/foo",
        "grpcs://example.com/foo",
        "custom://example.com/foo",
    ] {
        assert_eq!(
            helper_path(&provider, uri),
            Some(example_com_helper.clone()),
            "{uri}"
        );
    }

    for uri in [
        "https://subdomain.example.com/bar",
        "https://subdomain2.example.com/bar",
        "https://sub.subdomain.example.com/bar",
        "https://subdomain.example.com/bar",
    ] {
        assert_eq!(
            helper_path(&provider, uri),
            Some(example_com_wildcard_helper.clone()),
            "{uri}"
        );
    }

    assert_eq!(
        helper_path(&provider, "https://other-domain.com"),
        Some(default_helper)
    );
}

#[test]
fn prefer_most_specific_wildcard_match() {
    let fs = file_system();
    let example_com_wildcard_helper = path(&fs, EXAMPLE_COM_WILDCARD_HELPER_PATH);
    let sub_example_com_wildcard_helper = path(&fs, SUB_EXAMPLE_COM_WILDCARD_HELPER_PATH);
    let mut builder = CredentialHelperProvider::builder();
    builder
        .add("*.example.com", example_com_wildcard_helper.clone())
        .unwrap();
    builder
        .add("*.sub.example.com", sub_example_com_wildcard_helper.clone())
        .unwrap();
    let provider = builder.build();

    assert_eq!(
        helper_path(&provider, "https://example.com/bar"),
        Some(example_com_wildcard_helper.clone())
    );
    assert_eq!(
        helper_path(&provider, "https://foo.example.com/bar"),
        Some(example_com_wildcard_helper)
    );
    assert_eq!(
        helper_path(&provider, "https://sub.example.com/bar"),
        Some(sub_example_com_wildcard_helper.clone())
    );
    assert_eq!(
        helper_path(&provider, "https://foo.sub.example.com/bar"),
        Some(sub_example_com_wildcard_helper)
    );
}

#[test]
fn parent_domain_test() {
    // Java: parentDomain.
    assert_eq!(parent_domain("com"), None);

    assert_eq!(parent_domain("foo.example.com"), Some("example.com"));
    assert_eq!(parent_domain("example.com"), Some("com"));

    // Punycode URIs (münchen.de).
    assert_eq!(
        parent_domain("foo.xn--mnchen-3ya.de"),
        Some("xn--mnchen-3ya.de")
    );
    assert_eq!(
        parent_domain("bar.foo.xn--mnchen-3ya.de"),
        Some("foo.xn--mnchen-3ya.de")
    );
    assert_eq!(parent_domain("xn--mnchen-3ya.de"), Some("de"));
}
