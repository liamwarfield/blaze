//! `JdkServerCertVerifier` against the JDK's decisions on the pre-v3 server certificates of
//! `tests/data/jdk_cert_fixtures` (made by `scripts/jdk-certs/generate.py`; the verdicts in
//! `expected.tsv` were recorded with the oracle's embedded JDK 25.0.2 by `scripts/jdk-certs/run`,
//! which runs `CertificateFactory`, the PKIX trust manager and `HostnameChecker` as
//! `X509TrustManagerImpl.checkTrusted` does).
//!
//! The rule: Blaze accepts nothing the JDK rejects. Where the JDK accepts, Blaze accepts too,
//! except for the cases in [`STRICTER`], each a documented `PORT: stricter` of the verifier.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use blaze_remote::authandtls::jdk_server_cert_verifier::JdkServerCertVerifier;
use tokio_rustls::rustls::RootCertStore;
use tokio_rustls::rustls::client::danger::ServerCertVerifier;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};

/// Cases the JDK accepts and Blaze rejects, with the reason (see the verifier's `PORT:
/// stricter` comments).
const STRICTER: &[(&str, &str)] = &[
    (
        "alg_params_absent_inner",
        "signature algorithms compared by encoding",
    ),
    ("wildcard", "wildcard common names are not matched"),
    ("serial_leading_zero", "DER INTEGER required"),
    ("time_no_seconds", "UTCTime without seconds"),
    ("time_offset", "time zone other than Z"),
    ("time_fraction", "fractional seconds"),
    ("time_feb_31", "a day that is not in its month"),
    ("non_minimal_length", "DER lengths required"),
    ("trailing_bytes", "bytes after the certificate"),
    ("version_5", "version above 2 goes to webpki"),
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

/// 2026-11-26: inside every fixture's validity but the expired ones.
const NOW: u64 = 1_795_000_000;

fn blaze_accepts(dir: &Path, case: &str, roots_file: &str, host: &str) -> Result<(), String> {
    let mut roots = RootCertStore::empty();
    let pem = std::fs::read(dir.join(roots_file)).unwrap();
    for cert in CertificateDer::pem_slice_iter(&pem) {
        roots.add(cert.unwrap()).map_err(|e| format!("root: {e}"))?;
    }
    let verifier = JdkServerCertVerifier::new(
        Arc::new(roots),
        Arc::new(tokio_rustls::rustls::crypto::ring::default_provider()),
    )
    .unwrap()
    .with_requested_host(host);
    let der = CertificateDer::from(std::fs::read(dir.join(format!("{case}.der"))).unwrap());
    // A host name rustls cannot express is refused before the verifier runs.
    let server_name =
        ServerName::try_from(host.to_string()).map_err(|e| format!("server name: {e}"))?;
    verifier
        .verify_server_cert(
            &der,
            &[],
            &server_name,
            &[],
            UnixTime::since_unix_epoch(std::time::Duration::from_secs(NOW)),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
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
    assert_eq!(cases.len(), expected.len(), "every case has a JDK verdict");

    let mut failures = Vec::new();
    for row in &cases {
        let (case, roots, host) = (&row[0], &row[1], &row[2]);
        let (jdk, jdk_reason) = &expected[case];
        let blaze = blaze_accepts(&dir, case, roots, host);
        match (jdk.as_str(), &blaze) {
            ("REJECT", Ok(())) => failures.push(format!(
                "{case}: the JDK rejects ({jdk_reason}), Blaze accepts"
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
