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
//! PORT: rustls has no JDK trust manager. Every certificate whose version field is not 1 or 2
//! goes to rustls' `WebPkiServerVerifier`, unchanged. A v1 or v2 end-entity certificate is
//! checked here, as `X509TrustManagerImpl.checkTrusted` checks it: parsed as
//! `X509CertImpl`/`X509CertInfo` parse it, validated as `PKIXValidator` validates a
//! one-certificate chain, and its name matched as `HostnameChecker.matchDNS` matches it.
//!
//! The rule of this file: accept nothing the JDK rejects. Where an exact port would need more
//! than this verifier has (the JDK's BER leniency, the public suffix list, path building through
//! intermediates, `X500Principal` canonical name comparison), the check here is stricter, and
//! each such place says so (`PORT: stricter`). The JDK's decisions on the fixtures of
//! `tests/data/jdk_cert_fixtures` (recorded with the oracle's embedded JDK 25.0.2 by
//! `scripts/jdk-certs/run`) are the expected values of `tests/jdk_server_cert_fixtures_test.rs`.
//!
//! Java line numbers below are JDK 11's `sun.security` sources (the JDK 25 sources are not on
//! this machine); every check was confirmed on JDK 25.0.2 by a fixture, named next to it.
//!
//! Nothing in this file panics on certificate bytes: every parser returns `None` on malformed
//! input (`tests::malformed_inputs_never_panic`).

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
    /// The host name as the channel was given it. rustls' `ServerName` drops a trailing dot,
    /// which the JDK's `SNIHostName` refuses (`trailing_dot_host`).
    requested_host: Option<String>,
}

fn bad(e: CertificateError) -> Error {
    Error::InvalidCertificate(e)
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
            requested_host: None,
        })
    }

    /// The host name the channel verifies the server against, as written (see the field).
    pub fn with_requested_host(mut self, host: impl Into<String>) -> JdkServerCertVerifier {
        self.requested_host = Some(host.into());
        self
    }

    /// `PKIXValidator.engineValidate` for a chain whose first certificate is the pre-v3 `cert`,
    /// then `HostnameChecker.match` (see the module documentation).
    fn verify_pre_v3(
        &self,
        cert: &Certificate<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        now: UnixTime,
    ) -> Result<(), Error> {
        // Java (JDK 11): sun/security/validator/PKIXValidator.java:256 engineValidate: "Check if
        // chain[i] is already trusted": trustedCerts.contains(cert), or a trusted certificate
        // with the same subject and public key. For chain[0] the chain is returned as it is: no
        // signature or validity check (fixtures `direct`, `direct_expired`, `self_signed_expired`).
        // PORT: stricter: the subject is compared by its encoding, where Java compares
        // X500Principals (canonical form); the key by the contents of the subjectPublicKeyInfo,
        // where Java compares PublicKey.equals (re-encoded forms).
        let directly_trusted = self.roots.roots.iter().any(|anchor| {
            anchor.subject.as_ref() == cert.subject
                && anchor.subject_public_key_info.as_ref() == cert.spki.contents
        });

        if !directly_trusted {
            // PORT: stricter: a chain of more than one certificate whose first certificate is not
            // trusted itself goes to Java's ordering loop, partial validation or the certificate
            // path builder (PKIXValidator.java:243-306), which are not ported; it is rejected.
            if !intermediates.is_empty() {
                return Err(bad(CertificateError::UnknownIssuer));
            }
            // Java (JDK 11): PKIXValidator.java:269-275 "apparently issued by trust anchor?":
            // trustedSubjects.containsKey(issuer) && isSignatureValid(keys, last), then
            // doValidate: the PKIX CertPathValidator's BasicChecker checks the signature with the
            // anchor's key (`bad_signature`), the validity (`expired`) and the name chaining.
            // AlgorithmChecker: ring's algorithms (SHA-256 and up, RSA >= 2048 bits, P-256,
            // P-384, Ed25519) are all allowed by jdk.certpath.disabledAlgorithms. KeyChecker,
            // ConstraintsChecker and PolicyChecker have nothing to check on a pre-v3 end entity
            // (no extensions; the anchor carries no name constraints), and revocation is off
            // (PKIXValidator sets setRevocationEnabled(false)). Otherwise Java builds a path
            // (doBuild), which for one certificate finds the same anchors.
            // PORT: stricter: the issuer is compared with the anchor's subject by encoding.
            let issued_by_anchor = self.roots.roots.iter().any(|anchor| {
                anchor.subject.as_ref() == cert.issuer
                    && parse_spki_contents(anchor.subject_public_key_info.as_ref()).is_some_and(
                        |anchor_spki| {
                            verify_with(
                                self.supported.all,
                                &anchor_spki,
                                cert.signature_algorithm,
                                cert.tbs,
                                cert.signature,
                            )
                        },
                    )
            });
            if !issued_by_anchor {
                return Err(bad(CertificateError::UnknownIssuer));
            }
            // Java (JDK 11): sun/security/x509/CertificateValidity.java:271-275 valid(Date):
            // notBefore.after(now) is "not yet valid", notAfter.before(now) "expired", in
            // milliseconds.
            // PORT: stricter: rustls' time has whole seconds, so the second of notAfter itself
            // counts as expired (Java accepts it until its last millisecond).
            let now = now.as_secs() as i64;
            if now < cert.not_before {
                return Err(bad(CertificateError::NotValidYet));
            }
            if now >= cert.not_after {
                return Err(bad(CertificateError::Expired));
            }
        }

        // Java (JDK 11): sun/security/ssl/X509TrustManagerImpl.java:229 checkIdentity, with the
        // "HTTPS" algorithm grpc-netty sets: HostnameChecker.getInstance(TYPE_TLS).match.
        match server_name {
            ServerName::DnsName(name) => match_dns(
                self.requested_host.as_deref().unwrap_or(name.as_ref()),
                cert.subject,
            ),
            // Java (JDK 11): sun/security/util/HostnameChecker.java:100, 143 matchIP needs an
            // iPAddress subjectAltName, which a pre-v3 certificate cannot have (`ip_host`).
            _ => Err(bad(CertificateError::NotValidForName)),
        }
    }

    /// A handshake signature made with the key of a pre-v3 end-entity certificate.
    fn verify_pre_v3_signature(
        &self,
        message: &[u8],
        cert: &Certificate<'_>,
        scheme: SignatureScheme,
        signature: &[u8],
    ) -> Result<HandshakeSignatureValid, Error> {
        let algs = self
            .supported
            .mapping
            .iter()
            .find(|(s, _)| *s == scheme)
            .map(|(_, algs)| *algs)
            .ok_or(Error::PeerMisbehaved(
                tokio_rustls::rustls::PeerMisbehaved::SignedHandshakeWithUnadvertisedSigScheme,
            ))?;
        let spki =
            parse_spki_contents(cert.spki.contents).ok_or(bad(CertificateError::BadEncoding))?;
        for alg in algs {
            if alg.public_key_alg_id().as_ref() == spki.algorithm
                && alg
                    .verify_signature(spki.public_key, message, signature)
                    .is_ok()
            {
                return Ok(HandshakeSignatureValid::assertion());
            }
        }
        Err(bad(CertificateError::BadSignature))
    }
}

/// Which verifier a certificate goes to.
enum Route<'a> {
    /// The version field is absent, 0 or 1: parsed here; `None` if Java's parse fails.
    PreV3(Option<Certificate<'a>>),
    /// Anything else (v3, other versions, unreadable): webpki.
    Webpki,
}

fn route(der: &[u8]) -> Route<'_> {
    match certificate_version(der) {
        Some(0 | 1) => Route::PreV3(parse_pre_v3_certificate(der)),
        // PORT: stricter: Java parses a version field above 2 as a v2 certificate (`version_5`);
        // webpki rejects it.
        _ => Route::Webpki,
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
        match route(end_entity.as_ref()) {
            Route::PreV3(Some(cert)) => {
                self.verify_pre_v3(&cert, intermediates, server_name, now)?;
                Ok(ServerCertVerified::assertion())
            }
            // CertificateFactory.generateCertificate fails: the handshake fails.
            Route::PreV3(None) => Err(bad(CertificateError::BadEncoding)),
            Route::Webpki => self.webpki.verify_server_cert(
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
        match route(cert.as_ref()) {
            Route::PreV3(Some(parsed)) => {
                self.verify_pre_v3_signature(message, &parsed, dss.scheme, dss.signature())
            }
            Route::PreV3(None) => Err(bad(CertificateError::BadEncoding)),
            Route::Webpki => self.webpki.verify_tls12_signature(message, cert, dss),
        }
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        match route(cert.as_ref()) {
            Route::PreV3(Some(parsed)) => {
                tokio_rustls::rustls::crypto::verify_tls13_signature_with_raw_key(
                    message,
                    &SubjectPublicKeyInfoDer::from(parsed.spki.raw),
                    dss,
                    &self.supported,
                )
            }
            Route::PreV3(None) => Err(bad(CertificateError::BadEncoding)),
            Route::Webpki => self.webpki.verify_tls13_signature(message, cert, dss),
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
#[derive(Clone, Copy, Debug)]
struct Tlv<'a> {
    tag: u8,
    raw: &'a [u8],
    contents: &'a [u8],
}

/// Reads one DER element from the front of `input`: a low tag number and a definite length in
/// its shortest form.
///
/// Java (JDK 11): sun/security/util/DerInputStream.java getLength rejects a long form with a
/// leading zero byte or a value below 128, and more than 4 length bytes.
/// PORT: stricter: Java's X.509 parser also accepts indefinite lengths (BER) and, on JDK 25, a
/// long form for a short length (`non_minimal_length`); both are rejected here.
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
        if *input.get(2)? == 0 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | *input.get(2 + i)? as usize;
        }
        if len < 0x80 {
            return None;
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
const UTC_TIME: u8 = 0x17;
const GENERALIZED_TIME: u8 = 0x18;
/// `[0] EXPLICIT` (constructed, context-specific 0): the version.
const VERSION_TAG: u8 = 0xa0;
/// `[1] IMPLICIT` / `[2] IMPLICIT` UniqueIdentifier (primitive BIT STRING).
const ISSUER_UNIQUE_ID: u8 = 0x81;
const SUBJECT_UNIQUE_ID: u8 = 0x82;

/// The fields of an X.509 certificate this verifier uses.
struct Certificate<'a> {
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

/// The value of the `version` field (0 when absent), or `None` when the certificate cannot be
/// read that far. Decides the route only; [`parse_pre_v3_certificate`] parses.
fn certificate_version(der: &[u8]) -> Option<i64> {
    let (certificate, _) = expect(der, SEQUENCE)?;
    let (tbs, _) = expect(certificate.contents, SEQUENCE)?;
    if tbs.contents.first() != Some(&VERSION_TAG) {
        return Some(0);
    }
    let (explicit, _) = read_tlv(tbs.contents)?;
    let (v, rest) = expect(explicit.contents, INTEGER)?;
    if !rest.is_empty() {
        return None;
    }
    small_integer(v.contents)
}

/// The value of a DER INTEGER's contents if it fits in an `i64`; `None` for an empty or
/// non-minimal encoding.
///
/// Java (JDK 11): sun/security/util/DerInputBuffer.java:150-162 rejects an empty INTEGER and a
/// redundant leading zero.
/// PORT: stricter: also a redundant leading 0xff; JDK 25 accepts a redundant leading zero in a
/// certificate's serial number (`serial_leading_zero`), which is rejected here.
fn small_integer(contents: &[u8]) -> Option<i64> {
    if !minimal_integer(contents) || contents.len() > 8 {
        return None;
    }
    let mut value: i64 = if contents[0] & 0x80 != 0 { -1 } else { 0 };
    for &b in contents {
        value = (value << 8) | b as i64;
    }
    Some(value)
}

fn minimal_integer(contents: &[u8]) -> bool {
    match contents {
        [] => false,
        [0x00, b, ..] if b & 0x80 == 0 => false,
        [0xff, b, ..] if b & 0x80 != 0 => false,
        _ => true,
    }
}

/// An OBJECT IDENTIFIER's contents: non-empty, every subidentifier in its shortest form.
/// Java (JDK 11): sun/security/util/ObjectIdentifier.java check(byte[]).
fn valid_oid(contents: &[u8]) -> bool {
    if contents.is_empty() || contents[contents.len() - 1] & 0x80 != 0 {
        return false;
    }
    let mut start = true;
    for &b in contents {
        if start && b == 0x80 {
            return false;
        }
        start = b & 0x80 == 0;
    }
    true
}

/// An AlgorithmIdentifier: `SEQUENCE { OID, parameters OPTIONAL }` and nothing else.
/// Java (JDK 11): sun/security/x509/AlgorithmId.java:397-421 parse ("Invalid
/// AlgorithmIdentifier: extra data").
fn algorithm_identifier(input: &[u8]) -> Option<(Tlv<'_>, &[u8])> {
    let (alg, rest) = expect(input, SEQUENCE)?;
    let (oid, params) = expect(alg.contents, OID)?;
    if !valid_oid(oid.contents) {
        return None;
    }
    if !params.is_empty() {
        let (_, extra) = read_tlv(params)?;
        if !extra.is_empty() {
            return None;
        }
    }
    Some((alg, rest))
}

/// A Name: `SEQUENCE OF SET OF SEQUENCE { OID, value }`, every element well formed. Returns the
/// element; its contents are empty for an empty name.
/// Java (JDK 11): sun/security/x509/X500Name.java parseDER, RDN.java (`X500 RDN`), AVA.java
/// ("AVA not a sequence", "AVA, extra bytes").
/// PORT: stricter: an empty RDN (an empty SET) is rejected.
fn name(input: &[u8]) -> Option<(Tlv<'_>, &[u8])> {
    let (name, rest) = expect(input, SEQUENCE)?;
    let mut rdns = name.contents;
    while !rdns.is_empty() {
        let (rdn, more) = expect(rdns, SET)?;
        rdns = more;
        if rdn.contents.is_empty() {
            return None;
        }
        let mut avas = rdn.contents;
        while !avas.is_empty() {
            let (ava, more) = expect(avas, SEQUENCE)?;
            avas = more;
            let (oid, value) = expect(ava.contents, OID)?;
            if !valid_oid(oid.contents) {
                return None;
            }
            let (_, extra) = read_tlv(value)?;
            if !extra.is_empty() {
                return None;
            }
        }
    }
    Some((name, rest))
}

/// `X509CertImpl.parse` and `X509CertInfo.parse` of a certificate whose version is 1 or 2.
/// `None` where Java throws.
fn parse_pre_v3_certificate(der: &[u8]) -> Option<Certificate<'_>> {
    // Java (JDK 11): sun/security/x509/X509CertImpl.java:1810-1836 parse: a SEQUENCE of
    // exactly tbsCertificate, signatureAlgorithm, signatureValue ("signed overrun").
    // PORT: stricter: bytes after the certificate are rejected; Java's X509Factory reads one
    // DER block and ignores them (`trailing_bytes`).
    let (certificate, after) = expect(der, SEQUENCE)?;
    if !after.is_empty() {
        return None;
    }
    let (tbs, rest) = expect(certificate.contents, SEQUENCE)?;
    let (signature_algorithm, rest) = algorithm_identifier(rest)?;
    let (signature_value, rest) = expect(rest, BIT_STRING)?;
    if !rest.is_empty() {
        return None;
    }
    let signature = bit_string_bits(signature_value.contents)?;

    // Java (JDK 11): sun/security/x509/X509CertInfo.java:631-710 parse.
    let mut fields = tbs.contents;
    let mut version = 0;
    // :644 the version, [0] EXPLICIT INTEGER (CertificateVersion.java:80 "X.509 version, bad
    // format" for anything after the INTEGER).
    if fields.first() == Some(&VERSION_TAG) {
        let (explicit, rest) = read_tlv(fields)?;
        let (v, after) = expect(explicit.contents, INTEGER)?;
        if !after.is_empty() {
            return None;
        }
        version = small_integer(v.contents)?;
        fields = rest;
    }
    if !(0..=1).contains(&version) {
        return None;
    }
    // :650 serialNumber.
    let (serial, fields) = expect(fields, INTEGER)?;
    if !minimal_integer(serial.contents) {
        return None;
    }
    // :653 signature (the inner AlgorithmIdentifier).
    let (inner_signature_algorithm, fields) = algorithm_identifier(fields)?;
    // :656-660 issuer; "Empty issuer DN not allowed in X509Certificates" (`empty_issuer`).
    let (issuer, fields) = name(fields)?;
    if issuer.contents.is_empty() {
        return None;
    }
    // :663 validity (CertificateValidity.java:77-101: a SEQUENCE of exactly two times).
    let (validity, fields) = expect(fields, SEQUENCE)?;
    let (not_before, rest) = read_tlv(validity.contents)?;
    let (not_after, rest) = read_tlv(rest)?;
    if !rest.is_empty() {
        return None;
    }
    // :666-671 subject; "Empty subject DN not allowed in v1 certificate" (`empty_subject`).
    let (subject, fields) = name(fields)?;
    if version == 0 && subject.contents.is_empty() {
        return None;
    }
    // :674 subjectPublicKeyInfo (X509Key.parse: "excess subject key").
    let (spki, mut fields) = expect(fields, SEQUENCE)?;
    parse_spki_contents(spki.contents)?;
    // :677-681 "no more data allowed for version 1 certificate" (`v1_extensions`,
    // `v1_explicit_extensions`, `v1_unique_id`).
    if version == 0 && !fields.is_empty() {
        return None;
    }
    // :686-700 issuerUniqueID [1], then subjectUniqueID [2], each optional, in this order
    // (`v2_unique_ids`); :703-707 anything else in a v2 certificate is "Extensions not allowed
    // in v2 certificate" (`v2_extensions`, `v2_unique_ids_reversed`, `v2_trailing_integer`).
    // PORT: stricter: the unique identifiers must be primitive BIT STRINGs with a valid
    // unused-bits count.
    for tag in [ISSUER_UNIQUE_ID, SUBJECT_UNIQUE_ID] {
        if fields.first() == Some(&tag) {
            let (unique_id, rest) = read_tlv(fields)?;
            let (&unused, _) = unique_id.contents.split_first()?;
            if unused > 7 {
                return None;
            }
            fields = rest;
        }
    }
    if !fields.is_empty() {
        return None;
    }
    // X509CertImpl.java:1842-1847: "Signature algorithm mismatch" (`alg_mismatch`,
    // `alg_mismatch_outer`).
    // PORT: stricter: compared by encoding; Java's AlgorithmId.equals also takes absent
    // parameters as equal to NULL (`alg_params_absent_inner`).
    if inner_signature_algorithm.raw != signature_algorithm.raw {
        return None;
    }
    Some(Certificate {
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
    algorithm: &'a [u8],
    public_key: &'a [u8],
}

/// Parses the contents of a `subjectPublicKeyInfo` (as a trust anchor stores it): an
/// AlgorithmIdentifier and a BIT STRING, nothing else.
fn parse_spki_contents(contents: &[u8]) -> Option<Spki<'_>> {
    let (algorithm, rest) = algorithm_identifier(contents)?;
    let (key, rest) = expect(rest, BIT_STRING)?;
    if !rest.is_empty() {
        return None;
    }
    Some(Spki {
        algorithm: algorithm.contents,
        public_key: bit_string_bits(key.contents)?,
    })
}

/// The bits of a BIT STRING with no unused bits.
/// PORT: stricter: Java accepts unused bits (and masks them) in a signature or key.
fn bit_string_bits(contents: &[u8]) -> Option<&[u8]> {
    match contents.split_first() {
        Some((0, bits)) => Some(bits),
        _ => None,
    }
}

/// UTCTime (`YYMMDDHHMMSSZ`, years 1950 to 2049) or GeneralizedTime (`YYYYMMDDHHMMSSZ`), in
/// seconds since the epoch.
///
/// Java (JDK 11): sun/security/util/DerInputBuffer.java getTime: digits only
/// (`time_non_ascii`), month 1-12, day 1-31, hour < 24, minute and second < 60
/// (`time_second_60`), "Z" or an offset.
/// PORT: stricter: seconds are required, the zone must be "Z", no fraction, and the day must
/// exist in its month; Java also accepts YYMMDDhhmm, +hhmm/-hhmm offsets, fractions of a
/// GeneralizedTime and days up to 31 in any month (`time_no_seconds`, `time_offset`,
/// `time_fraction`, `time_feb_31`).
fn parse_time(tlv: Tlv<'_>) -> Option<i64> {
    let (zone, digits) = tlv.contents.split_last()?;
    if *zone != b'Z' || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        digits
            .get(range)?
            .iter()
            .try_fold(0i64, |n, d| Some(n * 10 + (d - b'0') as i64))
    };
    let (year, rest) = match (tlv.tag, digits.len()) {
        (UTC_TIME, 12) => {
            let yy = number(0..2)?;
            (if yy >= 50 { 1900 + yy } else { 2000 + yy }, 2)
        }
        (GENERALIZED_TIME, 14) => (number(0..4)?, 4),
        _ => return None,
    };
    let field = |i: usize| number(rest + i..rest + i + 2);
    let (month, day, hour, minute, second) =
        (field(0)?, field(2)?, field(4)?, field(6)?, field(8)?);
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86400 + hour * 3600 + minute * 60 + second)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
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

/// `X500Name.findMostSpecificAttribute(commonName_oid)` and `DerValue.getAsString()`: the first
/// commonName of the last RDN that has one (Java (JDK 11): sun/security/x509/X500Name.java:756-766,
/// RDN.java:324-331; `two_cn_first`, `two_cn_second`, `cn_last_rdn`, `cn_first_rdn`), as a
/// string; `None` if there is none or its value is not a string.
///
/// PORT: stricter: GeneralString and UniversalString values are not read (no match).
fn most_specific_common_name(name: &[u8]) -> Option<String> {
    const COMMON_NAME: [u8; 3] = [0x55, 0x04, 0x03];
    let mut found: Option<Tlv<'_>> = None;
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
            found = Some(value);
            // The first commonName of this RDN.
            break;
        }
    }
    let value = found?;
    match value.tag {
        // UTF8String
        0x0c => String::from_utf8(value.contents.to_vec()).ok(),
        // PrintableString, IA5String: ASCII.
        0x13 | 0x16 => value
            .contents
            .is_ascii()
            .then(|| value.contents.iter().map(|&b| b as char).collect()),
        // TeletexString: ISO-8859-1, as the JDK's DerValue reads it.
        0x14 => Some(value.contents.iter().map(|&b| b as char).collect()),
        // BMPString: UTF-16BE (`cn_bmp`).
        0x1e => {
            if value.contents.len() % 2 != 0 {
                return None;
            }
            let units = value
                .contents
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]));
            char::decode_utf16(units)
                .collect::<Result<String, _>>()
                .ok()
        }
        _ => None,
    }
}

/// `new SNIHostName(name)` succeeds: `IDN.toASCII(name, USE_STD3_ASCII_RULES)` (labels of
/// letters, digits and hyphens, not starting or ending with a hyphen, 1 to 63 characters) and
/// no trailing dot.
///
/// PORT: stricter: only ASCII names (Java converts internationalized names with IDN first).
// Java: javax/net/ssl/SNIHostName.java SNIHostName.<init>
fn is_sni_host_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
}

/// `HostnameChecker.matchDNS(expectedName, cert, chainsToPublicCA)` for a certificate without a
/// subjectAltName extension.
// Java (JDK 11): sun/security/util/HostnameChecker.java:189-236 HostnameChecker.matchDNS
fn match_dns(expected_name: &str, subject: &[u8]) -> Result<(), Error> {
    let not_valid = || bad(CertificateError::NotValidForName);
    // Check that the expected name is a valid domain name (`trailing_dot_host`, `underscore`).
    if !is_sni_host_name(expected_name) {
        return Err(not_valid());
    }
    // Java (JDK 11): HostnameChecker.java:100 isIpAddress: an IPv4 literal goes to matchIP,
    // which needs a subjectAltName.
    // PORT: stricter: every name whose labels are all digits is taken as one (Java's
    // IPAddressUtil.isIPv4LiteralAddress also checks the ranges).
    if expected_name
        .split('.')
        .all(|label| label.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(not_valid());
    }
    let Some(cname) = most_specific_common_name(subject) else {
        return Err(not_valid());
    };
    if is_matched(expected_name, &cname) {
        Ok(())
    } else {
        Err(not_valid())
    }
}

/// `HostnameChecker.isMatched(name, template, chainsToPublicCA)` for TYPE_TLS.
///
/// The template must be a valid SNI host name; then the names are compared label by label,
/// lower-cased (`v1_upper_host`). A template that is not NFKC-normalized is not matched; an
/// ASCII one always is.
/// PORT: stricter: a template with a wildcard is not matched (Java's matchAllWildcards and,
/// for an anchor among the JDK's public CAs, hasIllegalWildcard with the public suffix list are
/// not ported; Java accepts `*.example.com` for `foo.example.com` under a private CA,
/// `wildcard`).
// Java (JDK 11): sun/security/util/HostnameChecker.java:271-310 HostnameChecker.isMatched
fn is_matched(name: &str, template: &str) -> bool {
    if template.contains('*') {
        return false;
    }
    if !is_sni_host_name(template) {
        return false;
    }
    name.eq_ignore_ascii_case(template)
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
        assert_eq!(certificate_version(server.as_ref()), Some(0));
        let cert = parse_pre_v3_certificate(server.as_ref()).unwrap();
        // notBefore=Mar 26 20:18:14 2020 GMT, notAfter=May 28 20:18:14 3000 GMT
        assert_eq!(cert.not_before, 1_585_253_894);
        assert_eq!(cert.not_after, days_from_civil(3000, 5, 28) * 86400 + 73094);
        assert_eq!(
            most_specific_common_name(cert.subject).as_deref(),
            Some("localhost")
        );
        let ca = der(CA);
        assert_eq!(certificate_version(ca.as_ref()), Some(2));
        assert!(matches!(route(ca.as_ref()), Route::Webpki));
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
    fn not_after_second_counts_as_expired() {
        // Java compares milliseconds; rustls' time has seconds, so the last second is refused.
        let v = verifier(CA);
        let server = der(SERVER);
        let not_after = parse_pre_v3_certificate(server.as_ref()).unwrap().not_after as u64;
        let name = ServerName::try_from("localhost").unwrap();
        v.verify_server_cert(&server, &[], &name, &[], at(not_after - 1))
            .unwrap();
        assert_eq!(
            v.verify_server_cert(&server, &[], &name, &[], at(not_after))
                .err(),
            Some(Error::InvalidCertificate(CertificateError::Expired))
        );
    }

    #[test]
    fn a_chain_with_intermediates_is_not_built() {
        let v = verifier(CA);
        let name = ServerName::try_from("localhost").unwrap();
        assert_eq!(
            v.verify_server_cert(&der(SERVER), &[der(CA)], &name, &[], at(NOW))
                .err(),
            Some(Error::InvalidCertificate(CertificateError::UnknownIssuer))
        );
    }

    #[test]
    fn certificate_signature_is_checked_with_the_anchor_key() {
        let v = verifier(CA);
        let ca = der(CA);
        // The CA certificate is v3: read its key with the generic reader.
        let (ca_cert, _) = expect(ca.as_ref(), SEQUENCE).unwrap();
        let (ca_tbs, _) = expect(ca_cert.contents, SEQUENCE).unwrap();
        let mut fields = ca_tbs.contents;
        for _ in 0..6 {
            fields = read_tlv(fields).unwrap().1;
        }
        let (ca_spki, _) = expect(fields, SEQUENCE).unwrap();
        let anchor_spki = parse_spki_contents(ca_spki.contents).unwrap();
        let server = der(SERVER);
        let s = parse_pre_v3_certificate(server.as_ref()).unwrap();
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
        assert!(is_matched("localhost", "localhost"));
        assert!(is_matched("LocalHost", "localhost"));
        assert!(!is_matched("example.com", "localhost"));
        assert!(!is_matched("local_host", "local_host"));
        assert!(!is_matched("foo.example.com", "*.example.com"));
        assert!(is_sni_host_name("a-b.example.com"));
        assert!(!is_sni_host_name("localhost."));
        assert!(!is_sni_host_name("-a.example.com"));
        assert!(!is_sni_host_name("a..b"));
        assert!(!is_sni_host_name(&"a".repeat(64)));
    }

    /// `parse_time` on every malformed form, including a multi-byte character across the year
    /// boundary (Q2).
    #[test]
    fn malformed_times_are_rejected_without_panic() {
        let time = |tag: u8, s: &[u8]| {
            parse_time(Tlv {
                tag,
                raw: s,
                contents: s,
            })
        };
        assert_eq!(time(UTC_TIME, b"200326201814Z"), Some(1_585_253_894));
        for bad in [
            "2\u{e9}032620181Z".as_bytes(),
            "200\u{e9}26201814Z".as_bytes(),
            "20\u{e9}326201814Z".as_bytes(),
            b"",
            b"Z",
            b"200326201814",
            b"2003262018Z",
            b"200326201860Z",
            b"200326246014Z",
            b"201326201814Z",
            b"200231201814Z",
            b"200300201814Z",
            b"200326201814+0100",
            b"2003262018145Z",
            b"+00326201814Z",
            b"-00326201814Z",
        ] {
            assert_eq!(time(UTC_TIME, bad), None, "{bad:?}");
        }
        for bad in [
            "2\u{e9}0326201814Z".as_bytes(),
            "202\u{e9}0326201814Z".as_bytes(),
            b"20200326201814.5Z",
            b"2020032620181Z",
        ] {
            assert_eq!(time(GENERALIZED_TIME, bad), None, "{bad:?}");
        }
        assert_eq!(time(0x04, b"200326201814Z"), None);
    }

    /// Every parser of this file on truncated, extended and bit-flipped versions of a real
    /// certificate (Q2): none may panic.
    #[test]
    fn malformed_inputs_never_panic() {
        let v = verifier(CA);
        let server_name = ServerName::try_from("localhost").unwrap();
        for pem in [SERVER, CA] {
            let good = der(pem).as_ref().to_vec();
            let mut inputs: Vec<Vec<u8>> = Vec::new();
            for len in 0..=good.len() {
                inputs.push(good[..len].to_vec());
            }
            for i in 0..good.len() {
                for mask in [0x01u8, 0x80, 0xff] {
                    let mut flipped = good.clone();
                    flipped[i] ^= mask;
                    inputs.push(flipped);
                }
            }
            let mut extended = good.clone();
            extended.extend_from_slice(&[0x00, 0x00]);
            inputs.push(extended);
            for input in &inputs {
                let _ = certificate_version(input);
                let _ = parse_pre_v3_certificate(input);
                let _ = parse_spki_contents(input);
                let _ = name(input);
                let _ = algorithm_identifier(input);
                let _ = most_specific_common_name(input);
                let _ = read_tlv(input);
                let cert = CertificateDer::from(input.clone());
                let _ = v.verify_server_cert(&cert, &[], &server_name, &[], at(NOW));
                if let Route::PreV3(Some(parsed)) = route(input) {
                    for scheme in [
                        SignatureScheme::RSA_PKCS1_SHA256,
                        SignatureScheme::ECDSA_NISTP256_SHA256,
                    ] {
                        let _ = v.verify_pre_v3_signature(b"message", &parsed, scheme, &[0; 64]);
                    }
                }
            }
        }
        // The DER reader on hand-made headers.
        for input in [
            &[0x30, 0x84, 0xff, 0xff, 0xff, 0xff][..],
            &[0x30, 0x85, 0, 0, 0, 0, 1],
            &[0x30, 0x80],
            &[0x30, 0x81, 0x05, 0, 0, 0, 0, 0],
            &[0x30, 0x82, 0x00, 0x80],
            &[0x1f, 0x01, 0x00],
            &[0x30],
        ] {
            assert!(read_tlv(input).is_none(), "{input:?}");
        }
        assert_eq!(small_integer(&[]), None);
        assert_eq!(small_integer(&[0x00, 0x01]), None);
        assert_eq!(small_integer(&[0xff, 0x80]), None);
        assert_eq!(small_integer(&[0x01; 9]), None);
        assert_eq!(small_integer(&[0xff]), Some(-1));
        assert!(!valid_oid(&[]));
        assert!(!valid_oid(&[0x80, 0x01]));
        assert!(!valid_oid(&[0x2a, 0x86]));
        assert_eq!(
            most_specific_common_name(&[0x31, 0x03, 0x30, 0x01, 0x06]),
            None
        );
        // A BMPString of odd length or with a lone surrogate.
        let bmp_cn = |value: &[u8]| {
            let mut v = vec![0x31, (9 + value.len()) as u8, 0x30, (7 + value.len()) as u8];
            v.extend_from_slice(&[0x06, 0x03, 0x55, 0x04, 0x03, 0x1e, value.len() as u8]);
            v.extend_from_slice(value);
            v
        };
        assert_eq!(most_specific_common_name(&bmp_cn(&[0x00])), None);
        assert_eq!(most_specific_common_name(&bmp_cn(&[0xd8, 0x00])), None);
        assert_eq!(
            most_specific_common_name(&bmp_cn(&[0x00, 0x61])).as_deref(),
            Some("a")
        );
    }
}
