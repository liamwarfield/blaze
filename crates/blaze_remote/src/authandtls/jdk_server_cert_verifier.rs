//! The server certificate check of Bazel's gRPC channels for certificates that webpki cannot
//! parse.
//!
//! Java: `GoogleAuthUtils.createSSLContext` builds Netty's client context with
//! `GrpcSslContexts.forClient().trustManager(rootCert)`: the JDK's PKIX trust manager
//! (`TrustManagerFactory` over the given roots) and grpc-netty's endpoint identification
//! (`SSLParameters.setEndpointIdentificationAlgorithm("HTTPS")`, `sun.security.util.HostnameChecker`).
//! The JDK accepts an X.509 v1 end-entity certificate (Bazel's own test certificates
//! `src/test/testdata/test_tls_certificate/server.crt` and `client.crt` are v1); webpki rejects
//! every certificate that is not v3 (`UnsupportedCertVersion`).
//!
//! PORT: rustls has no JDK trust manager. Every v3 certificate goes to rustls'
//! `WebPkiServerVerifier`, unchanged. A v1 or v2 end-entity certificate (which carries no
//! extensions, hence no subjectAltName) is checked as PKIX checks a one-certificate path:
//! it is a trust anchor itself, or its issuer is a trust anchor whose key verifies its signature;
//! the current time is inside its validity; and the host name matches its most specific common
//! name (`HostnameChecker.matchDNS` without subjectAltName DNS entries). A path through an
//! intermediate certificate is not built for such a certificate (`UnknownIssuer`).

use std::sync::Arc;

use tokio_rustls::rustls::client::WebPkiServerVerifier;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use tokio_rustls::rustls::pki_types::{
    CertificateDer, ServerName, SignatureVerificationAlgorithm, SubjectPublicKeyInfoDer, UnixTime,
};
use tokio_rustls::rustls::{
    CertificateError, DigitallySignedStruct, Error, RootCertStore, SignatureScheme,
};

/// See the module documentation.
#[derive(Debug)]
pub struct JdkServerCertVerifier {
    webpki: Arc<WebPkiServerVerifier>,
    roots: Arc<RootCertStore>,
    supported: WebPkiSupportedAlgorithms,
}

impl JdkServerCertVerifier {
    pub fn new(
        roots: Arc<RootCertStore>,
        provider: Arc<CryptoProvider>,
    ) -> Result<JdkServerCertVerifier, Error> {
        let supported = provider.signature_verification_algorithms;
        let webpki = WebPkiServerVerifier::builder_with_provider(roots.clone(), provider)
            .build()
            .map_err(|e| Error::General(e.to_string()))?;
        Ok(JdkServerCertVerifier {
            webpki,
            roots,
            supported,
        })
    }

    /// The PKIX check of a v1/v2 end-entity certificate (see the module documentation).
    fn verify_pre_v3(
        &self,
        cert: &Certificate<'_>,
        server_name: &ServerName<'_>,
        now: UnixTime,
    ) -> Result<(), Error> {
        let bad = |e: CertificateError| Error::InvalidCertificate(e);
        let mut trusted = false;
        for anchor in self.roots.roots.iter() {
            let anchor_spki = parse_spki_contents(anchor.subject_public_key_info.as_ref())
                .ok_or(bad(CertificateError::BadEncoding))?;
            if anchor.subject.as_ref() == cert.subject && anchor_spki.raw == cert.spki.raw {
                // The certificate is itself a trust anchor.
                trusted = true;
                break;
            }
            if anchor.subject.as_ref() != cert.issuer {
                continue;
            }
            if verify_with(
                self.supported.all,
                &anchor_spki,
                cert.signature_algorithm,
                cert.tbs,
                cert.signature,
            ) {
                trusted = true;
                break;
            }
        }
        if !trusted {
            return Err(bad(CertificateError::UnknownIssuer));
        }
        let now = now.as_secs() as i64;
        if now < cert.not_before {
            return Err(bad(CertificateError::NotValidYet));
        }
        if now > cert.not_after {
            return Err(bad(CertificateError::Expired));
        }
        match server_name {
            ServerName::DnsName(name) => {
                // HostnameChecker.matchDNS: no subjectAltName (a pre-v3 certificate has no
                // extensions), so the most specific (last) common name of the subject.
                let Some(cn) = most_specific_common_name(cert.subject) else {
                    return Err(bad(CertificateError::NotValidForName));
                };
                if !host_matches(name.as_ref(), &cn) {
                    return Err(bad(CertificateError::NotValidForName));
                }
            }
            // HostnameChecker.matchIP needs an iPAddress subjectAltName.
            _ => return Err(bad(CertificateError::NotValidForName)),
        }
        Ok(())
    }

    /// A handshake signature made with the key of a pre-v3 end-entity certificate.
    fn verify_pre_v3_signature(
        &self,
        message: &[u8],
        cert: &Certificate<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        let algs = self
            .supported
            .mapping
            .iter()
            .find(|(scheme, _)| *scheme == dss.scheme)
            .map(|(_, algs)| *algs)
            .ok_or(Error::PeerMisbehaved(
                tokio_rustls::rustls::PeerMisbehaved::SignedHandshakeWithUnadvertisedSigScheme,
            ))?;
        let spki = parse_spki_contents(cert.spki.contents)
            .ok_or(Error::InvalidCertificate(CertificateError::BadEncoding))?;
        for alg in algs {
            if alg.public_key_alg_id().as_ref() == spki.algorithm
                && alg
                    .verify_signature(spki.public_key, message, dss.signature())
                    .is_ok()
            {
                return Ok(HandshakeSignatureValid::assertion());
            }
        }
        Err(Error::InvalidCertificate(CertificateError::BadSignature))
    }
}

impl ServerCertVerifier for JdkServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        match parse_certificate(end_entity.as_ref()) {
            Some(cert) if cert.version < 2 => {
                self.verify_pre_v3(&cert, server_name, now)?;
                Ok(ServerCertVerified::assertion())
            }
            _ => self.webpki.verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            ),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        match parse_certificate(cert.as_ref()) {
            Some(parsed) if parsed.version < 2 => {
                self.verify_pre_v3_signature(message, &parsed, dss)
            }
            _ => self.webpki.verify_tls12_signature(message, cert, dss),
        }
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        match parse_certificate(cert.as_ref()) {
            Some(parsed) if parsed.version < 2 => {
                tokio_rustls::rustls::crypto::verify_tls13_signature_with_raw_key(
                    message,
                    &SubjectPublicKeyInfoDer::from(parsed.spki.raw),
                    dss,
                    &self.supported,
                )
            }
            _ => self.webpki.verify_tls13_signature(message, cert, dss),
        }
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.webpki.supported_verify_schemes()
    }
}

/// Verifies `signature` over `message` with the key of `spki`, by the signature algorithm whose
/// AlgorithmIdentifier contents are `signature_algorithm`.
fn verify_with(
    algs: &[&dyn SignatureVerificationAlgorithm],
    spki: &Spki<'_>,
    signature_algorithm: &[u8],
    message: &[u8],
    signature: &[u8],
) -> bool {
    algs.iter().any(|alg| {
        alg.signature_alg_id().as_ref() == signature_algorithm
            && alg.public_key_alg_id().as_ref() == spki.algorithm
            && alg
                .verify_signature(spki.public_key, message, signature)
                .is_ok()
    })
}

/// A DER element: its full encoding and its contents.
#[derive(Clone, Copy)]
struct Tlv<'a> {
    tag: u8,
    raw: &'a [u8],
    contents: &'a [u8],
}

/// Reads one DER element (low tag numbers, definite lengths) from the front of `input`.
fn read_tlv(input: &[u8]) -> Option<(Tlv<'_>, &[u8])> {
    let tag = *input.first()?;
    if tag & 0x1f == 0x1f {
        return None;
    }
    let first = *input.get(1)? as usize;
    let (len, header) = if first < 0x80 {
        (first, 2)
    } else {
        let n = first & 0x7f;
        if n == 0 || n > 4 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | *input.get(2 + i)? as usize;
        }
        (len, 2 + n)
    };
    let end = header.checked_add(len)?;
    if end > input.len() {
        return None;
    }
    Some((
        Tlv {
            tag,
            raw: &input[..end],
            contents: &input[header..end],
        },
        &input[end..],
    ))
}

fn expect(input: &[u8], tag: u8) -> Option<(Tlv<'_>, &[u8])> {
    let (tlv, rest) = read_tlv(input)?;
    (tlv.tag == tag).then_some((tlv, rest))
}

const SEQUENCE: u8 = 0x30;
const SET: u8 = 0x31;
const INTEGER: u8 = 0x02;
const BIT_STRING: u8 = 0x03;
const OID: u8 = 0x06;

/// The fields of an X.509 certificate this verifier uses.
struct Certificate<'a> {
    /// 0 for v1, 1 for v2, 2 for v3 (the encoded `version`).
    version: u8,
    /// The full encoding of `tbsCertificate` (what the issuer signed).
    tbs: &'a [u8],
    /// The contents of the `signatureAlgorithm` AlgorithmIdentifier.
    signature_algorithm: &'a [u8],
    /// The bits of `signatureValue`.
    signature: &'a [u8],
    /// The contents of `issuer` and `subject` (Name, without the SEQUENCE header).
    issuer: &'a [u8],
    subject: &'a [u8],
    /// `subjectPublicKeyInfo`.
    spki: Tlv<'a>,
    /// Validity, in seconds since the epoch.
    not_before: i64,
    not_after: i64,
}

fn parse_certificate(der: &[u8]) -> Option<Certificate<'_>> {
    let (certificate, _) = expect(der, SEQUENCE)?;
    let (tbs, rest) = expect(certificate.contents, SEQUENCE)?;
    let (signature_algorithm, rest) = expect(rest, SEQUENCE)?;
    let (signature_value, _) = expect(rest, BIT_STRING)?;
    let signature = bit_string_bits(signature_value.contents)?;

    let mut fields = tbs.contents;
    let mut version = 0u8;
    if fields.first() == Some(&0xa0) {
        let (explicit, rest) = read_tlv(fields)?;
        let (v, _) = expect(explicit.contents, INTEGER)?;
        version = *v.contents.last()?;
        fields = rest;
    }
    let (_serial, fields) = expect(fields, INTEGER)?;
    let (_signature, fields) = expect(fields, SEQUENCE)?;
    let (issuer, fields) = expect(fields, SEQUENCE)?;
    let (validity, fields) = expect(fields, SEQUENCE)?;
    let (subject, fields) = expect(fields, SEQUENCE)?;
    let (spki, _) = expect(fields, SEQUENCE)?;
    let (not_before, rest) = read_tlv(validity.contents)?;
    let (not_after, _) = read_tlv(rest)?;
    Some(Certificate {
        version,
        tbs: tbs.raw,
        signature_algorithm: signature_algorithm.contents,
        signature,
        issuer: issuer.contents,
        subject: subject.contents,
        spki,
        not_before: parse_time(not_before)?,
        not_after: parse_time(not_after)?,
    })
}

/// A `subjectPublicKeyInfo`: the contents of its AlgorithmIdentifier and the key bits.
struct Spki<'a> {
    raw: &'a [u8],
    algorithm: &'a [u8],
    public_key: &'a [u8],
}

/// Parses the contents of a `subjectPublicKeyInfo` (as a trust anchor stores it).
fn parse_spki_contents(contents: &[u8]) -> Option<Spki<'_>> {
    let (algorithm, rest) = expect(contents, SEQUENCE)?;
    let (key, _) = expect(rest, BIT_STRING)?;
    Some(Spki {
        raw: contents,
        algorithm: algorithm.contents,
        public_key: bit_string_bits(key.contents)?,
    })
}

/// The bits of a BIT STRING with no unused bits.
fn bit_string_bits(contents: &[u8]) -> Option<&[u8]> {
    match contents.split_first() {
        Some((0, bits)) => Some(bits),
        _ => None,
    }
}

/// UTCTime (`YYMMDDHHMMSSZ`, years 1950 to 2049) or GeneralizedTime (`YYYYMMDDHHMMSSZ`), in
/// seconds since the epoch.
fn parse_time(tlv: Tlv<'_>) -> Option<i64> {
    let text = std::str::from_utf8(tlv.contents).ok()?;
    let text = text.strip_suffix('Z')?;
    let (year, rest) = match tlv.tag {
        0x17 if text.len() == 12 => {
            let yy: i64 = text[..2].parse().ok()?;
            (if yy >= 50 { 1900 + yy } else { 2000 + yy }, &text[2..])
        }
        0x18 if text.len() == 14 => (text[..4].parse().ok()?, &text[4..]),
        _ => return None,
    };
    if !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let field = |i: usize| -> i64 { rest[i..i + 2].parse().unwrap() };
    let (month, day, hour, minute, second) = (field(0), field(2), field(4), field(6), field(8));
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86400 + hour * 3600 + minute * 60 + second)
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The value of the last commonName attribute of a Name (`X500Name.findMostSpecificAttribute`).
fn most_specific_common_name(name: &[u8]) -> Option<String> {
    const COMMON_NAME: [u8; 3] = [0x55, 0x04, 0x03];
    let mut found = None;
    let mut rdns = name;
    while !rdns.is_empty() {
        let (rdn, rest) = expect(rdns, SET)?;
        rdns = rest;
        let mut attributes = rdn.contents;
        while !attributes.is_empty() {
            let (attribute, rest) = expect(attributes, SEQUENCE)?;
            attributes = rest;
            let (oid, rest) = expect(attribute.contents, OID)?;
            if oid.contents != COMMON_NAME {
                continue;
            }
            let (value, _) = read_tlv(rest)?;
            found = Some(match value.tag {
                // UTF8String, PrintableString, IA5String
                0x0c | 0x13 | 0x16 => String::from_utf8(value.contents.to_vec()).ok()?,
                // TeletexString: ISO-8859-1, as the JDK's DerValue reads it.
                0x14 => value.contents.iter().map(|&b| b as char).collect(),
                _ => return None,
            });
        }
    }
    found
}

/// `HostnameChecker.isMatched` for the HTTPS type, without wildcards: ASCII case-insensitive
/// (`equalsIgnoreCase` of the IDN-converted names; both are ASCII here).
// Java: sun/security/util/HostnameChecker.java HostnameChecker.isMatched
fn host_matches(host: &str, pattern: &str) -> bool {
    if pattern.contains('*') {
        // matchLeftmostWildcard and hasIllegalWildcard (with its public suffix list).
        blaze_util::unported!(
            "HostnameChecker.isMatched (a wildcard common name in a pre-v3 server certificate)"
        );
    }
    host.eq_ignore_ascii_case(pattern)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_rustls::rustls::pki_types::pem::PemObject;

    // Bazel's src/test/testdata/test_tls_certificate (ca.crt is v3, server.crt v1, both
    // CN=localhost, valid from 2020-03-26 to 3000-05-28).
    const CA: &str = include_str!("../../tests/data/test_tls_certificate/ca.crt");
    const SERVER: &str = include_str!("../../tests/data/test_tls_certificate/server.crt");

    fn der(pem: &str) -> CertificateDer<'static> {
        CertificateDer::from_pem_slice(pem.as_bytes()).unwrap()
    }

    fn verifier(root: &str) -> JdkServerCertVerifier {
        let mut roots = RootCertStore::empty();
        roots.add(der(root)).unwrap();
        JdkServerCertVerifier::new(
            Arc::new(roots),
            Arc::new(tokio_rustls::rustls::crypto::ring::default_provider()),
        )
        .unwrap()
    }

    fn at(secs: u64) -> UnixTime {
        UnixTime::since_unix_epoch(std::time::Duration::from_secs(secs))
    }

    const NOW: u64 = 1_795_000_000; // 2026-11

    #[test]
    fn parses_bazel_test_certificates() {
        let server = der(SERVER);
        let cert = parse_certificate(server.as_ref()).unwrap();
        assert_eq!(cert.version, 0);
        // notBefore=Mar 26 20:18:14 2020 GMT, notAfter=May 28 20:18:14 3000 GMT
        assert_eq!(cert.not_before, 1_585_253_894);
        assert_eq!(cert.not_after, days_from_civil(3000, 5, 28) * 86400 + 73094);
        assert_eq!(
            most_specific_common_name(cert.subject).as_deref(),
            Some("localhost")
        );
        let ca = der(CA);
        assert_eq!(parse_certificate(ca.as_ref()).unwrap().version, 2);
    }

    #[test]
    fn v1_server_certificate_signed_by_the_root_is_accepted() {
        let v = verifier(CA);
        let name = ServerName::try_from("localhost").unwrap();
        v.verify_server_cert(&der(SERVER), &[], &name, &[], at(NOW))
            .unwrap();
        let name = ServerName::try_from("LOCALHOST").unwrap();
        v.verify_server_cert(&der(SERVER), &[], &name, &[], at(NOW))
            .unwrap();
    }

    #[test]
    fn v1_server_certificate_wrong_name_is_rejected() {
        let v = verifier(CA);
        let name = ServerName::try_from("example.com").unwrap();
        assert_eq!(
            v.verify_server_cert(&der(SERVER), &[], &name, &[], at(NOW))
                .err(),
            Some(Error::InvalidCertificate(CertificateError::NotValidForName))
        );
        let ip = ServerName::try_from("127.0.0.1").unwrap();
        assert_eq!(
            v.verify_server_cert(&der(SERVER), &[], &ip, &[], at(NOW))
                .err(),
            Some(Error::InvalidCertificate(CertificateError::NotValidForName))
        );
    }

    #[test]
    fn v1_server_certificate_outside_validity_is_rejected() {
        let v = verifier(CA);
        let name = ServerName::try_from("localhost").unwrap();
        assert_eq!(
            v.verify_server_cert(&der(SERVER), &[], &name, &[], at(1_500_000_000))
                .err(),
            Some(Error::InvalidCertificate(CertificateError::NotValidYet))
        );
    }

    #[test]
    fn certificate_signature_is_checked_with_the_anchor_key() {
        let v = verifier(CA);
        let ca = der(CA);
        let anchor = parse_certificate(ca.as_ref()).unwrap();
        let anchor_spki = parse_spki_contents(anchor.spki.contents).unwrap();
        let server = der(SERVER);
        let s = parse_certificate(server.as_ref()).unwrap();
        assert!(verify_with(
            v.supported.all,
            &anchor_spki,
            s.signature_algorithm,
            s.tbs,
            s.signature
        ));
        let mut wrong = s.signature.to_vec();
        wrong[0] ^= 1;
        assert!(!verify_with(
            v.supported.all,
            &anchor_spki,
            s.signature_algorithm,
            s.tbs,
            &wrong
        ));
        // Signed by the CA, not by itself: with only its own key as anchor it is not trusted.
        let own_spki = parse_spki_contents(s.spki.contents).unwrap();
        assert!(!verify_with(
            v.supported.all,
            &own_spki,
            s.signature_algorithm,
            s.tbs,
            s.signature
        ));
    }

    #[test]
    fn v3_certificates_go_to_webpki() {
        // The CA certificate presented as the server's: webpki's answer (a CA certificate as an
        // end entity), not the pre-v3 path.
        let v = verifier(CA);
        let name = ServerName::try_from("localhost").unwrap();
        let ours = v.verify_server_cert(&der(CA), &[], &name, &[], at(NOW));
        let webpki = v
            .webpki
            .verify_server_cert(&der(CA), &[], &name, &[], at(NOW));
        assert!(webpki.is_err());
        assert_eq!(format!("{:?}", ours.err()), format!("{:?}", webpki.err()));
    }

    #[test]
    fn host_name_matching_follows_the_jdk() {
        assert!(host_matches("localhost", "localhost"));
        assert!(host_matches("LocalHost", "localhost"));
        assert!(!host_matches("example.com", "localhost"));
    }
}
