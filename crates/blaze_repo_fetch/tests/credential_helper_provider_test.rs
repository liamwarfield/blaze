//! Port of `src/test/java/com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperProviderTest.java`.
//!
//! Java runs the tests on an `InMemoryFileSystem` (not ported); here they run on the real file
//! system under a temporary directory, which prefixes Java's absolute paths.

use blaze_repo_fetch::authandtls::credentialhelper::CredentialHelperProvider;
use blaze_repo_fetch::uri::JavaUri;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};

const DEFAULT_HELPER_PATH: &str = "/path/to/default/helper";
const EXAMPLE_COM_HELPER_PATH: &str = "/path/to/example/com/helper";
const EXAMPLE_COM_WILDCARD_HELPER_PATH: &str = "/path/to/example/com/wildcard/helper";
const SUB_EXAMPLE_COM_WILDCARD_HELPER_PATH: &str = "/path/to/sub/example/com/wildcard/helper";

struct Fixture {
    _dir: tempfile::TempDir,
    file_system: FileSystemRef,
    root: String,
}

impl Fixture {
    // Java: CredentialHelperProviderTest.setUp
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let f = Fixture {
            _dir: dir,
            file_system: UnixFileSystem::create(DigestHashFunction::Sha256),
            root,
        };
        for path in [
            DEFAULT_HELPER_PATH,
            EXAMPLE_COM_HELPER_PATH,
            EXAMPLE_COM_WILDCARD_HELPER_PATH,
            SUB_EXAMPLE_COM_WILDCARD_HELPER_PATH,
        ] {
            f.set_up_helper(&f.path(path));
        }
        f
    }

    fn path(&self, path: &str) -> Path {
        Path::create_from_str(&format!("{}{}", self.root, path), &self.file_system)
    }

    fn set_up_helper(&self, path: &Path) {
        path.get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        // Just create an empty file, nothing to do.
        drop(path.get_output_stream().unwrap());
        path.set_executable(true).unwrap();
    }
}

fn uri(s: &str) -> JavaUri {
    JavaUri::create(s).unwrap()
}

fn helper_path(provider: &CredentialHelperProvider, u: &str) -> Path {
    provider
        .find_credential_helper(&uri(u))
        .unwrap()
        .get_path()
        .clone()
}

#[test]
fn no_helpers_configured() {
    let _f = Fixture::new();
    let provider = CredentialHelperProvider::builder().build();

    for u in [
        "http://example.com/foo",
        "https://example.com/foo",
        "grpc://example.com/foo",
        "grpcs://example.com/foo",
        "custom://example.com/foo",
        "https://subdomain.example.com/bar",
        "https://other-domain.com",
    ] {
        assert!(provider.find_credential_helper(&uri(u)).is_none(), "{u}");
    }
}

#[test]
fn uri_without_host_component() {
    let f = Fixture::new();
    let helper = f.path(EXAMPLE_COM_HELPER_PATH);
    let provider = CredentialHelperProvider::builder()
        .add_for_pattern("example.com", helper)
        .build();

    assert!(
        provider
            .find_credential_helper(&uri("unix:///path/to/socket"))
            .is_none()
    );
}

#[test]
fn only_default_helper() {
    let f = Fixture::new();
    let helper = f.path(DEFAULT_HELPER_PATH);
    let provider = CredentialHelperProvider::builder()
        .add(helper.clone())
        .build();

    for u in [
        "http://example.com/foo",
        "https://example.com/foo",
        "grpc://example.com/foo",
        "grpcs://example.com/foo",
        "unix:///tmp/grpc.sock",
        "custom://example.com/foo",
        "https://subdomain.example.com/bar",
        "https://other-domain.com",
    ] {
        assert_eq!(helper_path(&provider, u), helper, "{u}");
    }
}

#[test]
fn with_host_helpers_and_default_fallback() {
    let f = Fixture::new();
    let default_helper = f.path(DEFAULT_HELPER_PATH);
    let example_com_helper = f.path(EXAMPLE_COM_HELPER_PATH);
    let provider = CredentialHelperProvider::builder()
        .add(default_helper.clone())
        .add_for_pattern("example.com", example_com_helper.clone())
        .build();

    for u in [
        "http://example.com/foo",
        "https://example.com/foo",
        "grpc://example.com/foo",
        "grpcs://example.com/foo",
        "custom://example.com/foo",
    ] {
        assert_eq!(helper_path(&provider, u), example_com_helper, "{u}");
    }

    for u in [
        "https://subdomain.example.com/bar",
        "https://other-domain.com",
        "unix:///tmp/grpc.sock",
    ] {
        assert_eq!(helper_path(&provider, u), default_helper, "{u}");
    }
}

#[test]
fn wildcard_matching() {
    let f = Fixture::new();
    let default_helper = f.path(DEFAULT_HELPER_PATH);
    let example_com_wildcard_helper = f.path(EXAMPLE_COM_WILDCARD_HELPER_PATH);
    let provider = CredentialHelperProvider::builder()
        .add(default_helper.clone())
        .add_for_pattern("*.example.com", example_com_wildcard_helper.clone())
        .build();

    for u in [
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
            helper_path(&provider, u),
            example_com_wildcard_helper,
            "{u}"
        );
    }

    assert_eq!(
        helper_path(&provider, "https://other-domain.com"),
        default_helper
    );

    assert_eq!(
        helper_path(&provider, "unix:///tmp/grpc.sock"),
        default_helper
    );
}

#[test]
fn prefer_exact_match_over_wildcard_matching() {
    let f = Fixture::new();
    let default_helper = f.path(DEFAULT_HELPER_PATH);
    let example_com_helper = f.path(EXAMPLE_COM_HELPER_PATH);
    let example_com_wildcard_helper = f.path(EXAMPLE_COM_WILDCARD_HELPER_PATH);
    let provider = CredentialHelperProvider::builder()
        .add(default_helper.clone())
        .add_for_pattern("example.com", example_com_helper.clone())
        .add_for_pattern("*.example.com", example_com_wildcard_helper.clone())
        .build();

    for u in [
        "http://example.com/foo",
        "https://example.com/foo",
        "grpc://example.com/foo",
        "grpcs://example.com/foo",
        "custom://example.com/foo",
    ] {
        assert_eq!(helper_path(&provider, u), example_com_helper, "{u}");
    }

    for u in [
        "https://subdomain.example.com/bar",
        "https://subdomain2.example.com/bar",
        "https://sub.subdomain.example.com/bar",
        "https://subdomain.example.com/bar",
    ] {
        assert_eq!(
            helper_path(&provider, u),
            example_com_wildcard_helper,
            "{u}"
        );
    }

    assert_eq!(
        helper_path(&provider, "https://other-domain.com"),
        default_helper
    );
}

#[test]
fn prefer_most_specific_wildcard_match() {
    let f = Fixture::new();
    let example_com_wildcard_helper = f.path(EXAMPLE_COM_WILDCARD_HELPER_PATH);
    let sub_example_com_wildcard_helper = f.path(SUB_EXAMPLE_COM_WILDCARD_HELPER_PATH);
    let provider = CredentialHelperProvider::builder()
        .add_for_pattern("*.example.com", example_com_wildcard_helper.clone())
        .add_for_pattern("*.sub.example.com", sub_example_com_wildcard_helper.clone())
        .build();

    assert_eq!(
        helper_path(&provider, "https://example.com/bar"),
        example_com_wildcard_helper
    );
    assert_eq!(
        helper_path(&provider, "https://foo.example.com/bar"),
        example_com_wildcard_helper
    );
    assert_eq!(
        helper_path(&provider, "https://sub.example.com/bar"),
        sub_example_com_wildcard_helper
    );
    assert_eq!(
        helper_path(&provider, "https://foo.sub.example.com/bar"),
        sub_example_com_wildcard_helper
    );
}

#[test]
fn parent_domain() {
    assert_eq!(CredentialHelperProvider::parent_domain("com"), None);

    assert_eq!(
        CredentialHelperProvider::parent_domain("foo.example.com"),
        Some("example.com")
    );
    assert_eq!(
        CredentialHelperProvider::parent_domain("example.com"),
        Some("com")
    );

    // Punycode URIs (münchen.de).
    assert_eq!(
        CredentialHelperProvider::parent_domain("foo.xn--mnchen-3ya.de"),
        Some("xn--mnchen-3ya.de")
    );
    assert_eq!(
        CredentialHelperProvider::parent_domain("bar.foo.xn--mnchen-3ya.de"),
        Some("foo.xn--mnchen-3ya.de")
    );
    assert_eq!(
        CredentialHelperProvider::parent_domain("xn--mnchen-3ya.de"),
        Some("de")
    );
}
