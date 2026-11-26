//! The server certificate check of Bazel's gRPC channels.
//!
//! Java: `GoogleAuthUtils.createSSLContext` builds Netty's client context with
//! `GrpcSslContexts.forClient().trustManager(rootCert)`: the JDK's PKIX trust manager over the
//! given roots and grpc-netty's endpoint identification (`HostnameChecker`, type TLS).
//!
//! PORT (ruling D16, docs/decisions.md): Blaze uses rustls' webpki verifier, with exactly one
//! more trust rule, the JDK's direct trust: an end-entity certificate that is byte-identical (the
//! whole DER) to a certificate of the configured trust file (`--tls_certificate`) is trusted
//! without building a chain, whatever its version (`PKIXValidator.engineValidate` returns
//! `chain[0]` when `trustedCerts.contains(cert)`, which compares encodings). The host name is then
//! checked: through webpki's subjectAltName check for a v3 certificate, and for a v1/v2
//! certificate against the first commonName of the last RDN that has one (`HostnameChecker`
//! without subjectAltName). The handshake signature is verified with that certificate's key.
//!
//! Declared deviations (stricter than Bazel, fail closed):
//! - Blaze does not validate chains of X.509 v1/v2 certificates. A v1/v2 server certificate that
//!   is not byte-identical to a trusted one is refused ([`PRE_V3_CHAIN_NOT_SUPPORTED`]); the JDK
//!   validates it against the roots.
//! - A directly trusted v3 certificate without a subjectAltName dNSName is refused (webpki has
//!   no commonName fallback; the JDK matches the CN).
//! - Every v3 chain is webpki's: webpki refuses some end entities the JDK accepts under a
//!   private CA (CN only, `CA:TRUE`).
//! - Signature algorithms: the subset of the provider's that the oracle's TLS stack accepts
//!   ([`oracle_signature_verification_algorithms`]).
//!
//! Nothing in this file panics on certificate bytes; a v1/v2 certificate this file cannot fully
//! read is refused.

use std::sync::{Arc, OnceLock};

use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::client::{WebPkiServerVerifier, verify_server_name};
use tokio_rustls::rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use tokio_rustls::rustls::pki_types::{
    CertificateDer, ServerName, SignatureVerificationAlgorithm, SubjectPublicKeyInfoDer, UnixTime,
    alg_id,
};
use tokio_rustls::rustls::server::ParsedCertificate;
use tokio_rustls::rustls::{
    CertificateError, DigitallySignedStruct, Error, OtherError, RootCertStore, SignatureScheme,
};

/// Blaze's error text for a v1/v2 server certificate that is not trusted directly (ruling D16).
pub const PRE_V3_CHAIN_NOT_SUPPORTED: &str = "X.509 v1/v2 server certificate chains are not \
     supported by Blaze: a v1/v2 server certificate is trusted only if it is identical to a \
     certificate in --tls_certificate";

/// See the module documentation.
#[derive(Debug)]
pub struct JdkServerCertVerifier {
    /// `None` when the trust file holds no certificate webpki takes as an anchor (only v1/v2
    /// ones): then only direct trust can succeed.
    webpki: Option<Arc<WebPkiServerVerifier>>,
    /// The certificates of the configured trust file, for the direct trust rule; empty for the
    /// built-in roots.
    trusted_certificates: Vec<CertificateDer<'static>>,
    supported: WebPkiSupportedAlgorithms,
    /// The host name as the channel was given it. rustls' `ServerName` drops a trailing dot,
    /// which the JDK's `SNIHostName` refuses.
    requested_host: Option<String>,
}

fn bad(e: CertificateError) -> Error {
    Error::InvalidCertificate(e)
}

/// The signature algorithms of the oracle's TLS client, as far as the provider has them.
///
/// The oracle (Bazel 9.3.2) runs grpc-netty over netty-tcnative (BoringSSL); its session
/// advertises and its JDK trust manager accepts the signature algorithms of
/// `ExtendedOpenSslSession.LOCAL_SUPPORTED_SIGNATURE_ALGORITHMS` (Netty 4.1.119.Final, in
/// `A-server.jar`): SHA512withRSA, SHA512withECDSA, SHA384withRSA, SHA384withECDSA,
/// SHA256withRSA, SHA256withECDSA, SHA224withRSA, SHA224withECDSA, SHA1withRSA, SHA1withECDSA,
/// RSASSA-PSS. The provider's set (ring: RSA PKCS#1 and RSA-PSS with SHA-256/384/512, ECDSA P-256
/// and P-384 with SHA-256/384, Ed25519) minus Ed25519 lies inside that list; Ed25519 is removed.
/// PORT: stricter: SHA-1 and SHA-224 signatures, which the oracle accepts, are not in the
/// provider.
// Java: io/netty/handler/ssl/ExtendedOpenSslSession.java ExtendedOpenSslSession.LOCAL_SUPPORTED_SIGNATURE_ALGORITHMS
pub fn oracle_signature_verification_algorithms(
    provider: &WebPkiSupportedAlgorithms,
) -> WebPkiSupportedAlgorithms {
    static FILTERED: OnceLock<WebPkiSupportedAlgorithms> = OnceLock::new();
    *FILTERED.get_or_init(|| {
        let keep = |alg: &&'static dyn SignatureVerificationAlgorithm| {
            alg.public_key_alg_id() != alg_id::ED25519
        };
        let all: Vec<&'static dyn SignatureVerificationAlgorithm> =
            provider.all.iter().copied().filter(keep).collect();
        let mapping: Vec<(
            SignatureScheme,
            &'static [&'static dyn SignatureVerificationAlgorithm],
        )> = provider
            .mapping
            .iter()
            .filter(|(scheme, _)| *scheme != SignatureScheme::ED25519)
            .map(|(scheme, algs)| {
                let algs: Vec<&'static dyn SignatureVerificationAlgorithm> =
                    algs.iter().copied().filter(keep).collect();
                (*scheme, &*Box::leak(algs.into_boxed_slice()))
            })
            .filter(|(_, algs)| !algs.is_empty())
            .collect();
        WebPkiSupportedAlgorithms {
            all: Box::leak(all.into_boxed_slice()),
            mapping: Box::leak(mapping.into_boxed_slice()),
        }
    })
}

impl JdkServerCertVerifier {
    /// `roots` are the trust anchors (the trust file's certificates, or the built-in roots);
    /// `trusted_certificates` the certificates of the trust file, for direct trust.
    pub fn new(
        roots: Arc<RootCertStore>,
        trusted_certificates: Vec<CertificateDer<'static>>,
        provider: Arc<CryptoProvider>,
    ) -> Result<JdkServerCertVerifier, Error> {
        let supported =
            oracle_signature_verification_algorithms(&provider.signature_verification_algorithms);
        let provider = Arc::new(CryptoProvider {
            signature_verification_algorithms: supported,
            ..(*provider).clone()
        });
        let webpki = if roots.is_empty() {
            None
        } else {
            Some(
                WebPkiServerVerifier::builder_with_provider(roots, provider)
                    .build()
                    .map_err(|e| Error::General(e.to_string()))?,
            )
        };
        Ok(JdkServerCertVerifier {
            webpki,
            trusted_certificates,
            supported,
            requested_host: None,
        })
    }

    /// The host name the channel verifies the server against, as written (see the field).
    pub fn with_requested_host(mut self, host: impl Into<String>) -> JdkServerCertVerifier {
        self.requested_host = Some(host.into());
        self
    }

    /// `trustedCerts.contains(chain[0])`: byte equality of the whole certificate.
    // Java: sun/security/validator/PKIXValidator.java:234 PKIXValidator.engineValidate
    fn is_directly_trusted(&self, end_entity: &CertificateDer<'_>) -> bool {
        self.trusted_certificates
            .iter()
            .any(|trusted| trusted.as_ref() == end_entity.as_ref())
    }

    /// `HostnameChecker.match` for a directly trusted certificate.
    // Java: sun/security/util/HostnameChecker.java HostnameChecker.match
    fn check_host_name(
        &self,
        end_entity: &CertificateDer<'_>,
        server_name: &ServerName<'_>,
    ) -> Result<(), Error> {
        match pre_v3_parts(end_entity.as_ref()) {
            Route::PreV3(Some(parts)) => match server_name {
                ServerName::DnsName(name) => match_dns(
                    self.requested_host.as_deref().unwrap_or(name.as_ref()),
                    parts.subject,
                ),
                // HostnameChecker.matchIP needs an iPAddress subjectAltName.
                _ => Err(bad(CertificateError::NotValidForName)),
            },
            Route::PreV3(None) => Err(bad(CertificateError::BadEncoding)),
            Route::Other => {
                // The requested host must be a valid SNI host name (`SNIHostName`), which rustls'
                // `ServerName` no longer shows for a trailing dot.
                if let (ServerName::DnsName(_), Some(host)) = (server_name, &self.requested_host)
                    && !is_sni_host_name(host)
                {
                    return Err(bad(CertificateError::NotValidForName));
                }
                let parsed = ParsedCertificate::try_from(end_entity)?;
                verify_server_name(&parsed, server_name)
            }
        }
    }

    /// A handshake signature made with the key of a v1/v2 certificate (webpki cannot read one).
    fn verify_pre_v3_signature(
        &self,
        message: &[u8],
        parts: &PreV3Parts<'_>,
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
        for alg in algs {
            if alg.public_key_alg_id().as_ref() == parts.spki_algorithm
                && alg
                    .verify_signature(parts.public_key, message, signature)
                    .is_ok()
            {
                return Ok(HandshakeSignatureValid::assertion());
            }
        }
        Err(bad(CertificateError::BadSignature))
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
        // Direct trust: no chain, no validity check (Java returns chain[0] unchecked).
        if self.is_directly_trusted(end_entity) {
            self.check_host_name(end_entity, server_name)?;
            return Ok(ServerCertVerified::assertion());
        }
        match pre_v3_parts(end_entity.as_ref()) {
            // PORT: ruling D16: no chain validation for v1/v2 certificates.
            // (rustls prints `CertificateError::Other` in debug form; `Error::Other` prints the
            // message: "other error: <message>".)
            Route::PreV3(_) => Err(Error::Other(OtherError(Arc::new(std::io::Error::other(
                PRE_V3_CHAIN_NOT_SUPPORTED,
            ))))),
            Route::Other => match &self.webpki {
                Some(webpki) => webpki.verify_server_cert(
                    end_entity,
                    intermediates,
                    server_name,
                    ocsp_response,
                    now,
                ),
                None => Err(bad(CertificateError::UnknownIssuer)),
            },
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        match pre_v3_parts(cert.as_ref()) {
            Route::PreV3(Some(parts)) => {
                self.verify_pre_v3_signature(message, &parts, dss.scheme, dss.signature())
            }
            Route::PreV3(None) => Err(bad(CertificateError::BadEncoding)),
            Route::Other => tokio_rustls::rustls::crypto::verify_tls12_signature(
                message,
                cert,
                dss,
                &self.supported,
            ),
        }
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        match pre_v3_parts(cert.as_ref()) {
            Route::PreV3(Some(parts)) => {
                tokio_rustls::rustls::crypto::verify_tls13_signature_with_raw_key(
                    message,
                    &SubjectPublicKeyInfoDer::from(parts.spki_raw),
                    dss,
                    &self.supported,
                )
            }
            Route::PreV3(None) => Err(bad(CertificateError::BadEncoding)),
            Route::Other => tokio_rustls::rustls::crypto::verify_tls13_signature(
                message,
                cert,
                dss,
                &self.supported,
            ),
        }
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported.supported_schemes()
    }
}

/// A DER element: its full encoding and its contents.
#[derive(Clone, Copy, Debug)]
struct Tlv<'a> {
    tag: u8,
    raw: &'a [u8],
    contents: &'a [u8],
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
/// `[0] EXPLICIT` (constructed, context-specific 0): the version.
const VERSION_TAG: u8 = 0xa0;

/// What this file reads of a v1/v2 certificate.
struct PreV3Parts<'a> {
    /// The contents of `subject` (Name, without the SEQUENCE header).
    subject: &'a [u8],
    /// The full `subjectPublicKeyInfo`.
    spki_raw: &'a [u8],
    /// The contents of its AlgorithmIdentifier.
    spki_algorithm: &'a [u8],
    /// The key bits.
    public_key: &'a [u8],
}

/// Which way a certificate goes.
enum Route<'a> {
    /// The version field is absent, 0 or 1: the parts this file reads, or `None` if it cannot
    /// read them.
    PreV3(Option<PreV3Parts<'a>>),
    /// Anything else (v3, other versions, unreadable): webpki.
    Other,
}

/// The version of a certificate, then for v1/v2 the subject and the key.
///
/// Only the structure up to the key is read: trust comes from byte equality with a trusted
/// certificate, not from this parse. Every element read must be well-formed DER; anything else
/// is `PreV3(None)`.
fn pre_v3_parts(der: &[u8]) -> Route<'_> {
    let Some((certificate, after)) = expect(der, SEQUENCE) else {
        return Route::Other;
    };
    let Some((tbs, _)) = expect(certificate.contents, SEQUENCE) else {
        return Route::Other;
    };
    let mut fields = tbs.contents;
    if fields.first() == Some(&VERSION_TAG) {
        let Some((explicit, rest)) = read_tlv(fields) else {
            return Route::Other;
        };
        let version = match expect(explicit.contents, INTEGER) {
            Some((v, [])) => small_integer(v.contents),
            _ => None,
        };
        if !matches!(version, Some(0 | 1)) {
            return Route::Other;
        }
        fields = rest;
    }
    if !after.is_empty() {
        return Route::PreV3(None);
    }
    Route::PreV3(read_pre_v3_fields(fields))
}

/// serialNumber, signature, issuer, validity, subject, subjectPublicKeyInfo.
fn read_pre_v3_fields(fields: &[u8]) -> Option<PreV3Parts<'_>> {
    let (serial, fields) = expect(fields, INTEGER)?;
    if !minimal_integer(serial.contents) {
        return None;
    }
    let (_signature, fields) = algorithm_identifier(fields)?;
    let (_issuer, fields) = name(fields)?;
    let (_validity, fields) = expect(fields, SEQUENCE)?;
    let (subject, fields) = name(fields)?;
    let (spki, _) = expect(fields, SEQUENCE)?;
    let (algorithm, rest) = algorithm_identifier(spki.contents)?;
    let (key, rest) = expect(rest, BIT_STRING)?;
    if !rest.is_empty() {
        return None;
    }
    Some(PreV3Parts {
        subject: subject.contents,
        spki_raw: spki.raw,
        spki_algorithm: algorithm.contents,
        public_key: bit_string_bits(key.contents)?,
    })
}

/// An AlgorithmIdentifier: `SEQUENCE { OID, parameters OPTIONAL }` and nothing else.
fn algorithm_identifier(input: &[u8]) -> Option<(Tlv<'_>, &[u8])> {
    let (alg, rest) = expect(input, SEQUENCE)?;
    let (_oid, params) = expect(alg.contents, OID)?;
    if !params.is_empty() {
        let (_, extra) = read_tlv(params)?;
        if !extra.is_empty() {
            return None;
        }
    }
    Some((alg, rest))
}

/// A Name: `SEQUENCE OF SET OF SEQUENCE { OID, value }`, every element well-formed DER, no
/// empty RDN.
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
            let (_oid, value) = expect(ava.contents, OID)?;
            let (_, extra) = read_tlv(value)?;
            if !extra.is_empty() {
                return None;
            }
        }
    }
    Some((name, rest))
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

/// The bits of a BIT STRING with no unused bits.
/// PORT: stricter: Java accepts unused bits (and masks them) in a signature or key.
fn bit_string_bits(contents: &[u8]) -> Option<&[u8]> {
    match contents.split_first() {
        Some((0, bits)) => Some(bits),
        _ => None,
    }
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

    // Bazel's src/test/testdata/test_tls_certificate (ca.crt is v3, server.crt v1 signed by it,
    // both CN=localhost).
    const CA: &str = include_str!("../../tests/data/test_tls_certificate/ca.crt");
    const SERVER: &str = include_str!("../../tests/data/test_tls_certificate/server.crt");

    fn der(pem: &str) -> CertificateDer<'static> {
        CertificateDer::from_pem_slice(pem.as_bytes()).unwrap()
    }

    /// The verifier of a channel whose `--tls_certificate` holds `trust_file`.
    fn verifier(trust_file: &[&str]) -> JdkServerCertVerifier {
        let mut roots = RootCertStore::empty();
        let certs: Vec<CertificateDer<'static>> = trust_file.iter().map(|p| der(p)).collect();
        roots.add_parsable_certificates(certs.clone());
        JdkServerCertVerifier::new(
            Arc::new(roots),
            certs,
            Arc::new(tokio_rustls::rustls::crypto::ring::default_provider()),
        )
        .unwrap()
    }

    fn now() -> UnixTime {
        UnixTime::since_unix_epoch(std::time::Duration::from_secs(1_795_000_000))
    }

    fn localhost() -> ServerName<'static> {
        ServerName::try_from("localhost").unwrap()
    }

    #[test]
    fn routes_bazel_test_certificates() {
        assert!(matches!(
            pre_v3_parts(der(SERVER).as_ref()),
            Route::PreV3(Some(_))
        ));
        assert!(matches!(pre_v3_parts(der(CA).as_ref()), Route::Other));
        let server = der(SERVER);
        let Route::PreV3(Some(parts)) = pre_v3_parts(server.as_ref()) else {
            unreachable!()
        };
        assert_eq!(
            most_specific_common_name(parts.subject).as_deref(),
            Some("localhost")
        );
    }

    #[test]
    fn v1_certificate_signed_by_a_trusted_root_is_refused() {
        // Ruling D16: no chain validation for v1/v2 certificates.
        let v = verifier(&[CA]);
        let e = v
            .verify_server_cert(&der(SERVER), &[], &localhost(), &[], now())
            .unwrap_err();
        assert!(e.to_string().contains(PRE_V3_CHAIN_NOT_SUPPORTED), "{e}");
    }

    #[test]
    fn v1_certificate_identical_to_a_trusted_one_is_accepted() {
        let v = verifier(&[CA, SERVER]);
        v.verify_server_cert(&der(SERVER), &[], &localhost(), &[], now())
            .unwrap();
        let upper = ServerName::try_from("LOCALHOST").unwrap();
        v.verify_server_cert(&der(SERVER), &[], &upper, &[], now())
            .unwrap();
        // The host name is still checked.
        let other = ServerName::try_from("example.com").unwrap();
        assert_eq!(
            v.verify_server_cert(&der(SERVER), &[], &other, &[], now())
                .err(),
            Some(bad(CertificateError::NotValidForName))
        );
        let ip = ServerName::try_from("127.0.0.1").unwrap();
        assert_eq!(
            v.verify_server_cert(&der(SERVER), &[], &ip, &[], now())
                .err(),
            Some(bad(CertificateError::NotValidForName))
        );
        // No validity check under direct trust (the JDK returns chain[0] unchecked).
        v.verify_server_cert(
            &der(SERVER),
            &[],
            &localhost(),
            &[],
            UnixTime::since_unix_epoch(std::time::Duration::from_secs(0)),
        )
        .unwrap();
    }

    #[test]
    fn direct_trust_compares_the_whole_certificate() {
        // A certificate that differs from the trusted one in one byte of its signature: same
        // subject and key, not the same certificate (T1).
        let v = verifier(&[SERVER]);
        let mut twin = der(SERVER).as_ref().to_vec();
        let last = twin.len() - 1;
        twin[last] ^= 1;
        let e = v
            .verify_server_cert(&CertificateDer::from(twin), &[], &localhost(), &[], now())
            .unwrap_err();
        assert!(e.to_string().contains(PRE_V3_CHAIN_NOT_SUPPORTED), "{e}");
    }

    #[test]
    fn requested_host_with_trailing_dot_is_refused() {
        let v = verifier(&[SERVER]).with_requested_host("localhost.");
        assert_eq!(
            v.verify_server_cert(&der(SERVER), &[], &localhost(), &[], now())
                .err(),
            Some(bad(CertificateError::NotValidForName))
        );
    }

    #[test]
    fn ed25519_is_not_offered() {
        let v = verifier(&[CA]);
        assert!(
            !v.supported_verify_schemes()
                .contains(&SignatureScheme::ED25519)
        );
        assert!(
            v.supported
                .all
                .iter()
                .all(|alg| alg.public_key_alg_id() != alg_id::ED25519)
        );
        assert!(
            v.supported_verify_schemes()
                .contains(&SignatureScheme::RSA_PSS_SHA256)
        );
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

    /// Every reader of this file on truncated, extended and bit-flipped versions of real
    /// certificates: none may panic.
    #[test]
    fn malformed_inputs_never_panic() {
        let v = verifier(&[CA, SERVER]);
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
                let _ = name(input);
                let _ = algorithm_identifier(input);
                let _ = most_specific_common_name(input);
                let _ = read_tlv(input);
                let cert = CertificateDer::from(input.clone());
                let _ = v.verify_server_cert(&cert, &[], &localhost(), &[], now());
                let _ = v.check_host_name(&cert, &localhost());
                if let Route::PreV3(Some(parts)) = pre_v3_parts(input) {
                    for scheme in [
                        SignatureScheme::RSA_PKCS1_SHA256,
                        SignatureScheme::ECDSA_NISTP256_SHA256,
                    ] {
                        let _ = v.verify_pre_v3_signature(b"message", &parts, scheme, &[0; 64]);
                    }
                }
            }
        }
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
