//! `JdkServerCertVerifier` against the JDK's decisions on the server certificates of
//! `tests/data/jdk_cert_fixtures` (made by `scripts/jdk-certs/generate.py`; the verdicts in
//! `expected.tsv` were recorded with the oracle's embedded JDK 25.0.2 by `scripts/jdk-certs/run`,
//! which runs `CertificateFactory`, the PKIX trust manager and `HostnameChecker` as
//! `X509TrustManagerImpl.checkTrusted` does).
//!
//! Each case is a full TLS handshake (TLS 1.3, then TLS 1.2) between a rustls client with the
//! verifier and a rustls server presenting the case's certificate and signing with
//! `keys/ee.key` (or the key the case names), so the end entity's key is part of the decision, as
//! it is in Bazel's handshake.
//!
//! The rule: Blaze accepts nothing the JDK rejects, except for the cases in [`LENIENT`] (ruling
//! D16). Where the JDK accepts, Blaze accepts too, except for the cases in [`STRICTER`], each a
//! declared deviation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use blaze_remote::authandtls::jdk_server_cert_verifier::JdkServerCertVerifier;
use tokio_rustls::rustls::crypto::CryptoProvider;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use tokio_rustls::rustls::server::{ClientHello, ResolvesServerCert};
use tokio_rustls::rustls::sign::CertifiedKey;
use tokio_rustls::rustls::{ClientConfig, RootCertStore, ServerConfig, SupportedProtocolVersion};

/// Cases the JDK (as `JdkCertProbe` asks it) accepts and Blaze refuses, with the reason.
const STRICTER: &[(&str, &str)] = &[
    ("direct_wildcard", "wildcard common names are not matched"),
    (
        "ee_key_unknown_alg",
        "the handshake signature cannot be verified with the certificate's key (Bazel's TLS \
         engine fails the handshake too; the probe has no handshake)",
    ),
    ("chain_v1", "ruling D16: no chains of v1/v2 certificates"),
    (
        "v3_self_ca_cn_only",
        "webpki's name check needs a subjectAltName (the JDK falls back to the CN)",
    ),
    (
        "v3_self_noca_cn_only",
        "webpki's name check needs a subjectAltName (the JDK falls back to the CN)",
    ),
    (
        "v3_ed25519_ca",
        "Ed25519 is not in the oracle's TLS signature list (Netty's \
         LOCAL_SUPPORTED_SIGNATURE_ALGORITHMS); the probe has no TLS engine",
    ),
    (
        "direct_key_unused_bits",
        "a key BIT STRING with unused bits is not read (DER rules)",
    ),
    ("direct_ber_length", "BER (indefinite) lengths are not read"),
    ("v3_rsa1024_ca", "webpki refuses RSA keys under 2048 bits"),
    (
        "v3_sha1_signed",
        "SHA-1 signatures are not in the provider (the oracle accepts them)",
    ),
    (
        "v3_p521_ca",
        "P-521 is not in the provider (the oracle accepts it)",
    ),
    (
        "direct_non_minimal_length",
        "a long-form length for a short value is not read (DER rules; JDK 25 accepts it)",
    ),
    (
        "direct_serial_leading_zero",
        "a serial number with a redundant leading zero is not read (DER rules; JDK 25 accepts \
         it)",
    ),
    (
        "cn_general",
        "a GeneralString commonName is not read (no match)",
    ),
    (
        "cn_universal",
        "a UniversalString commonName is not read (no match)",
    ),
    (
        "direct_idn_host",
        "only ASCII host names: rustls' ServerName refuses an internationalized name before the \
         verifier (the JDK converts it with IDN)",
    ),
    (
        "direct_all_digit_host",
        "a host whose labels are all digits is taken as an IP address (rustls' ServerName \
         refuses 256.0.0.1; the JDK's isIPv4LiteralAddress checks the ranges and matches it as \
         a DNS name)",
    ),
];

/// Cases the JDK rejects and Blaze accepts: ruling D16 (docs/decisions.md), as root generalised it
/// in review round 7 (T8). A certificate that is both the user's own trust file and the server's
/// certificate, byte for byte, is trusted by identity: of a directly trusted v1/v2 certificate
/// Blaze makes the structural check of the TBS (up to the key, then only the unique identifiers
/// of v2) and nothing more. Every other reason the JDK's parser would refuse it (Bazel refuses to
/// start with such a trust file) is a declared lenient case, one fixture per class: the validity
/// contents, the outer algorithm identifier, an empty issuer, unique-identifier contents, the
/// outer structure after the TBS, OID contents.
const LENIENT: &[(&str, &str)] = &[
    (
        "direct_time_second_60",
        "D16: the validity of a directly trusted certificate is not read",
    ),
    (
        "direct_alg_mismatch",
        "D16: the outer signature algorithm of a directly trusted certificate is not read",
    ),
    (
        "direct_empty_issuer",
        "D16: the issuer of a directly trusted certificate is not checked for emptiness",
    ),
    (
        "direct_v2_issuer_uid_empty",
        "D16 (T8): the contents of a unique identifier are not read (81 00)",
    ),
    (
        "direct_v2_issuer_uid_bad_padding",
        "D16 (T8): the contents of a unique identifier are not read (81 01 09)",
    ),
    (
        "direct_outer_tbs_only",
        "D16 (T8): nothing after the TBS is read (no signature algorithm, no signature)",
    ),
    (
        "direct_outer_fourth_element",
        "D16 (T8): nothing after the TBS is read (a fourth element)",
    ),
    (
        "direct_outer_signature_octet_string",
        "D16 (T8): nothing after the TBS is read (a signature that is not a BIT STRING)",
    ),
    (
        "direct_inner_alg_empty_oid",
        "D16 (T8): OID contents are not read (06 00 as the inner signature algorithm)",
    ),
    (
        "direct_name_attribute_empty_oid",
        "D16 (T8): OID contents are not read (06 00 as a name attribute type)",
    ),
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/jdk_cert_fixtures")
}

fn read_tsv(path: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect()
}

/// The server's certificate, whatever it holds, with `keys/ee.key` as the signing key.
#[derive(Debug)]
struct FixedCert(Arc<CertifiedKey>);

impl ResolvesServerCert for FixedCert {
    fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.clone())
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(tokio_rustls::rustls::crypto::ring::default_provider())
}

/// One handshake; `Err` is the client's failure.
fn handshake(
    dir: &Path,
    case: &str,
    roots_file: &str,
    host: &str,
    key_file: &str,
    version: &'static SupportedProtocolVersion,
) -> Result<(), String> {
    let pem = std::fs::read(dir.join(roots_file)).unwrap();
    let trusted: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&pem)
        .collect::<Result<_, _>>()
        .unwrap();
    let mut roots = RootCertStore::empty();
    roots.add_parsable_certificates(trusted.clone());
    let verifier = JdkServerCertVerifier::new(Arc::new(roots), trusted, provider())
        .unwrap()
        .with_requested_host(host);
    let client_config = ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[version])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    // A host name rustls cannot express is refused before the verifier runs.
    let server_name =
        ServerName::try_from(host.to_string()).map_err(|e| format!("server name: {e}"))?;

    let cert = CertificateDer::from(std::fs::read(dir.join(format!("{case}.der"))).unwrap());
    let key =
        PrivateKeyDer::from_pem_slice(&std::fs::read(dir.join("keys").join(key_file)).unwrap())
            .unwrap();
    let signing_key = provider().key_provider.load_private_key(key).unwrap();
    let server_config = ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[version])
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(FixedCert(Arc::new(CertifiedKey::new(
            vec![cert],
            signing_key,
        )))));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let (client_io, server_io) = tokio::io::duplex(1 << 16);
        let connector = tokio_rustls::TlsConnector::from(Arc::new(client_config));
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
        let server = async move {
            // The server's side ends when the client aborts; its result is not the verdict.
            let _ = acceptor.accept(server_io).await;
        };
        let client = connector.connect(server_name, client_io);
        let (client, ()) = tokio::join!(client, server);
        client.map(|_| ()).map_err(|e| e.to_string())
    })
}

#[test]
fn verdicts_match_the_jdk() {
    let dir = fixtures();
    let cases = read_tsv(&dir.join("cases.tsv"));
    let expected: HashMap<String, (String, String)> = read_tsv(&dir.join("expected.tsv"))
        .into_iter()
        .map(|row| {
            (
                row[0].clone(),
                (row[1].clone(), row.get(2).cloned().unwrap_or_default()),
            )
        })
        .collect();
    let stricter: HashMap<&str, &str> = STRICTER.iter().copied().collect();
    let lenient: HashMap<&str, &str> = LENIENT.iter().copied().collect();
    assert_eq!(cases.len(), expected.len(), "every case has a JDK verdict");

    let mut failures = Vec::new();
    for row in &cases {
        let (case, roots, host) = (&row[0], &row[1], &row[2]);
        let key = row.get(4).map(String::as_str).unwrap_or("ee.key");
        let (jdk, jdk_reason) = &expected[case];
        let tls13 = handshake(
            &dir,
            case,
            roots,
            host,
            key,
            &tokio_rustls::rustls::version::TLS13,
        );
        let tls12 = handshake(
            &dir,
            case,
            roots,
            host,
            key,
            &tokio_rustls::rustls::version::TLS12,
        );
        if tls13.is_ok() != tls12.is_ok() {
            failures.push(format!(
                "{case}: TLS 1.3 {tls13:?} and TLS 1.2 {tls12:?} disagree"
            ));
            continue;
        }
        match (jdk.as_str(), &tls13) {
            ("REJECT", Ok(())) if !lenient.contains_key(case.as_str()) => failures.push(format!(
                "{case}: the JDK rejects ({jdk_reason}), Blaze accepts"
            )),
            ("REJECT", Err(e)) if lenient.contains_key(case.as_str()) => failures.push(format!(
                "{case}: listed as lenient (D16), but Blaze rejects as the JDK does: {e}"
            )),
            ("ACCEPT", Err(e)) if !stricter.contains_key(case.as_str()) => {
                failures.push(format!("{case}: the JDK accepts, Blaze rejects: {e}"))
            }
            ("ACCEPT", Ok(())) if stricter.contains_key(case.as_str()) => failures.push(format!(
                "{case}: listed as stricter, but Blaze accepts as the JDK does"
            )),
            _ => {}
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The refusal of a v1 chain carries Blaze's own text (ruling D16).
#[test]
fn v1_chain_is_refused_with_blazes_text() {
    let e = handshake(
        &fixtures(),
        "chain_v1",
        "ca.pem",
        "localhost",
        "ee.key",
        &tokio_rustls::rustls::version::TLS13,
    )
    .unwrap_err();
    assert!(
        e.contains("X.509 v1/v2 server certificate chains are not supported"),
        "{e}"
    );
}
