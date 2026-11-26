#!/usr/bin/env python3
"""Generates the pre-v3 server certificate fixtures of blaze_remote's JdkServerCertVerifier.

usage: generate.py <out_dir>
       generate.py help

Writes into <out_dir>:
  keys/ca.key, keys/ee.key, keys/other.key   RSA-2048 keys (PEM, PKCS#8), reused when present
  ca.pem          a v3 CA certificate (CN=Blaze Test CA), the trust anchor of most cases
  <case>.der      one end-entity certificate per case (DER, built by hand so that malformed
                  structures can be expressed)
  <case>.root.pem the trust anchors of a case when they are not ca.pem
  cases.tsv       case, roots file, host name, description (input of JdkCertProbe)

The verdicts are not written here: scripts/jdk-certs/run records what the oracle's JDK decides
for each case (expected.tsv).
"""

import base64
import datetime
import os
import sys

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec, ed25519, padding, rsa
from cryptography.x509.oid import NameOID


def length(n, long_form=False):
    if n < 0x80 and not long_form:
        return bytes([n])
    b = n.to_bytes(max(1, (n.bit_length() + 7) // 8), "big")
    return bytes([0x80 | len(b)]) + b


def tlv(tag, content, long_form=False):
    return bytes([tag]) + length(len(content), long_form) + content


def seq(*items):
    return tlv(0x30, b"".join(items))


def set_(*items):
    return tlv(0x31, b"".join(items))


def integer(value_bytes):
    return tlv(0x02, value_bytes)


def oid(dotted):
    parts = [int(p) for p in dotted.split(".")]
    out = bytearray([parts[0] * 40 + parts[1]])
    for p in parts[2:]:
        chunk = [p & 0x7F]
        p >>= 7
        while p:
            chunk.append(0x80 | (p & 0x7F))
            p >>= 7
        out += bytes(reversed(chunk))
    return tlv(0x06, bytes(out))


NULL = b"\x05\x00"
SHA256_RSA = "1.2.840.113549.1.1.11"
SHA384_RSA = "1.2.840.113549.1.1.12"
CN = "2.5.4.3"
O = "2.5.4.10"


def alg(dotted, params=NULL):
    return seq(oid(dotted), params)


def printable(s):
    return tlv(0x13, s.encode("ascii"))


def bmp(s):
    return tlv(0x1E, s.encode("utf-16-be"))


def ava(type_oid, value):
    return seq(oid(type_oid), value)


def name(*rdns):
    """rdns: lists of (oid, encoded value)."""
    return seq(*[set_(*[ava(t, v) for t, v in rdn]) for rdn in rdns])


def cn_name(cn):
    return name([(CN, printable(cn))])


def utc(s):
    return tlv(0x17, s.encode("latin-1") if isinstance(s, str) else s)


def gen(s):
    return tlv(0x18, s.encode("ascii"))


VALID = seq(utc("200326201814Z"), gen("29990528201814Z"))
EXPIRED = seq(utc("000101000000Z"), utc("010101000000Z"))


def bit_string(b):
    return tlv(0x03, b"\x00" + b)


def load_or_create_key(path):
    if os.path.exists(path):
        with open(path, "rb") as f:
            return serialization.load_pem_private_key(f.read(), None)
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    with open(path, "wb") as f:
        f.write(
            key.private_bytes(
                serialization.Encoding.PEM,
                serialization.PrivateFormat.PKCS8,
                serialization.NoEncryption(),
            )
        )
    return key



def load_or_create_ed25519_key(path):
    if os.path.exists(path):
        with open(path, "rb") as f:
            return serialization.load_pem_private_key(f.read(), None)
    key = ed25519.Ed25519PrivateKey.generate()
    with open(path, "wb") as f:
        f.write(key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8,
                                  serialization.NoEncryption()))
    return key


def load_or_create_p256_key(path):
    if os.path.exists(path):
        with open(path, "rb") as f:
            return serialization.load_pem_private_key(f.read(), None)
    key = ec.generate_private_key(ec.SECP256R1())
    with open(path, "wb") as f:
        f.write(key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8,
                                  serialization.NoEncryption()))
    return key


def ca_certificate(key, cn, digest):
    n = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, cn)])
    return (x509.CertificateBuilder().subject_name(n).issuer_name(n).public_key(key.public_key())
            .serial_number(2).not_valid_before(datetime.datetime(2020, 1, 1))
            .not_valid_after(datetime.datetime(2999, 1, 1))
            .add_extension(x509.BasicConstraints(ca=True, path_length=None), critical=True)
            .sign(key, digest))


def spki(key):
    return key.public_key().public_bytes(
        serialization.Encoding.DER, serialization.PublicFormat.SubjectPublicKeyInfo
    )


def sign(key, tbs, digest=hashes.SHA256):
    return key.sign(tbs, padding.PKCS1v15(), digest())


def certificate(
    signer,
    issuer,
    subject,
    subject_key,
    version=None,
    serial=b"\x01\x02",
    inner_alg=None,
    outer_alg=None,
    validity=VALID,
    trailing=b"",
    after_cert=b"",
    digest=hashes.SHA256,
):
    inner_alg = inner_alg if inner_alg is not None else alg(SHA256_RSA)
    outer_alg = outer_alg if outer_alg is not None else alg(SHA256_RSA)
    fields = b""
    if version is not None:
        fields += version
    fields += integer(serial) + inner_alg + issuer + validity + subject + spki(subject_key)
    fields += trailing
    tbs = seq(fields)
    return seq(tbs, outer_alg, bit_string(sign(signer, tbs, digest))) + after_cert


def explicit_version(n):
    return tlv(0xA0, integer(bytes([n])))


def main(argv):
    if len(argv) != 2 or argv[1] in ("help", "-h", "--help"):
        print(__doc__.strip())
        return 0 if len(argv) == 2 else 2
    out = argv[1]
    os.makedirs(os.path.join(out, "keys"), exist_ok=True)
    ca_key = load_or_create_key(os.path.join(out, "keys", "ca.key"))
    ee_key = load_or_create_key(os.path.join(out, "keys", "ee.key"))
    other_key = load_or_create_key(os.path.join(out, "keys", "other.key"))

    ca_subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "Blaze Test CA")])
    ca = (
        x509.CertificateBuilder()
        .subject_name(ca_subject)
        .issuer_name(ca_subject)
        .public_key(ca_key.public_key())
        .serial_number(1)
        .not_valid_before(datetime.datetime(2020, 1, 1))
        .not_valid_after(datetime.datetime(2999, 1, 1))
        .add_extension(x509.BasicConstraints(ca=True, path_length=None), critical=True)
        .add_extension(
            x509.KeyUsage(
                digital_signature=True,
                content_commitment=False,
                key_encipherment=False,
                data_encipherment=False,
                key_agreement=False,
                key_cert_sign=True,
                crl_sign=True,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .sign(ca_key, hashes.SHA256())
    )
    with open(os.path.join(out, "ca.pem"), "wb") as f:
        f.write(ca.public_bytes(serialization.Encoding.PEM))
    ca_name = ca_subject.public_bytes()

    cases = []

    def case(case_name, der, description, host="localhost", roots=None):
        """`roots=None`: the trust file is the certificate itself (direct trust)."""
        with open(os.path.join(out, case_name + ".der"), "wb") as f:
            f.write(der)
        if roots is None:
            roots = root_file(case_name, der)
        cases.append((case_name, roots, host, description))

    def root_file(case_name, *ders):
        path = case_name + ".root.pem"
        with open(os.path.join(out, path), "w") as f:
            for der in ders:
                f.write(pem_of(der))
        return path

    def pem_of(der):
        body = base64.encodebytes(der).decode()
        return "-----BEGIN CERTIFICATE-----\n" + body + "-----END CERTIFICATE-----\n"

    ee = lambda **kw: certificate(ca_key, ca_name, cn_name("localhost"), ee_key, **kw)
    with_subject = lambda subject, **kw: certificate(ca_key, ca_name, subject, ee_key, **kw)

    # Direct trust: the trust file holds the server's certificate itself. The decision points
    # left are the byte comparison and the host name check (ruling D16).
    case("direct_v1", ee(), "v1, trusted directly")
    case("direct_v1_upper_host", ee(), "v1, trusted directly, host in upper case",
         host="LOCALHOST")
    case("direct_v1_explicit", ee(version=explicit_version(0)), "version [0] INTEGER 0 encoded")
    case("direct_v2", ee(version=explicit_version(1)), "version 2")
    case("direct_v1_expired", ee(validity=EXPIRED), "v1, trusted directly, expired")
    case("direct_wrong_host", ee(), "CN=localhost, host example.com", host="example.com")
    case("direct_ip_host", with_subject(cn_name("127.0.0.1")), "CN and host 127.0.0.1",
         host="127.0.0.1")
    case("direct_trailing_dot_host", ee(), "host localhost.", host="localhost.")
    case("direct_underscore", with_subject(cn_name("local_host")), "CN and host local_host",
         host="local_host")
    case("direct_wildcard", with_subject(cn_name("*.example.com")),
         "CN=*.example.com, host foo.example.com", host="foo.example.com")
    case("two_cn_first", with_subject(name([(CN, printable("localhost")), (CN, printable("other"))])),
         "one RDN, CN=localhost then CN=other (as encoded)")
    case("two_cn_second", with_subject(name([(CN, printable("other")), (CN, printable("localhost"))])),
         "one RDN, CN=other then CN=localhost (as encoded)")
    case("cn_last_rdn", with_subject(name([(CN, printable("other"))], [(O, printable("x"))],
                                         [(CN, printable("localhost"))])),
         "CN=other, O=x, CN=localhost in three RDNs")
    case("cn_first_rdn", with_subject(name([(CN, printable("localhost"))], [(CN, printable("other"))])),
         "CN=localhost, CN=other in two RDNs")
    case("cn_bmp", with_subject(name([(CN, bmp("localhost"))])), "CN as a BMPString")
    for n, tag in (("cn_utf8", 0x0C), ("cn_t61", 0x14), ("cn_ia5", 0x16)):
        case(n, with_subject(name([(CN, tlv(tag, b"localhost"))])), n)
    case("cn_first_ava_other_type",
         with_subject(name([(CN, tlv(0x04, b"x")), (CN, printable("localhost"))])),
         "one RDN: CN OCTET STRING then CN=localhost")
    case("rdn_unsorted", with_subject(name([(O, printable("zz")), (CN, printable("localhost"))])),
         "multi-valued RDN not in DER SET order")
    case("subject_empty_value", with_subject(name([(O, tlv(0x13, b""))], [(CN, printable("localhost"))])),
         "O with an empty PrintableString")
    case("subject_constructed_value",
         with_subject(name([(O, tlv(0x30, b""))], [(CN, printable("localhost"))])),
         "O with a SEQUENCE value")
    case("subject_printable_bad",
         with_subject(name([(O, tlv(0x13, b"a@\xff"))], [(CN, printable("localhost"))])),
         "O PrintableString with non-printable bytes")
    case("subject_utf8_bad",
         with_subject(name([(O, tlv(0x0C, b"\xff\xfe"))], [(CN, printable("localhost"))])),
         "O UTF8String invalid")
    case("empty_subject_v2", with_subject(seq(), version=explicit_version(1)), "v2, empty subject")
    self_signed = certificate(ee_key, cn_name("localhost"), cn_name("localhost"), ee_key)
    case("self_signed", self_signed, "self-signed v1, trusted directly")

    # The end entity's key (the handshake signature is made with keys/ee.key).
    class RawKey:
        def __init__(self, der):
            self.der = der

        def public_key(self):
            return self

        def public_bytes(self, *a):
            return self.der

    good_spki = spki(ee_key)
    key_bits = good_spki[good_spki.index(b"\x03\x82"):]
    case("ee_key_alg_bad_null",
         certificate(ca_key, ca_name, cn_name("localhost"),
                     RawKey(seq(seq(oid("1.2.840.113549.1.1.1"), b"\x05\x01\x00"), key_bits))),
         "key AlgorithmIdentifier with NULL of length 1")
    case("ee_key_unknown_alg",
         certificate(ca_key, ca_name, cn_name("localhost"),
                     RawKey(seq(seq(oid("1.2.3.4.5")), bit_string(b"\x01\x02")))),
         "key of an unknown algorithm")
    case("ee_key_garbage_rsa",
         certificate(ca_key, ca_name, cn_name("localhost"),
                     RawKey(seq(alg("1.2.840.113549.1.1.1"), bit_string(b"\x01\x02")))),
         "rsaEncryption with garbage key bits")

    # Not identical to a trusted certificate.
    other_name = cn_name("Other Issuer")
    anchor = certificate(other_key, other_name, cn_name("localhost"), ee_key)
    twin = certificate(other_key, cn_name("Nobody"), cn_name("localhost"), ee_key,
                       validity=EXPIRED, serial=b"\x07")
    case("direct_twin", twin, "same subject and key as the trusted certificate, other bytes",
         roots=root_file("direct_twin", anchor))
    case("chain_v1", ee(), "v1 issued by the trusted CA (D16: no v1 chains)", roots="ca.pem")
    case("self_signed_untrusted", self_signed, "self-signed v1, not trusted", roots="ca.pem")

    # v3: webpki, plus direct trust.
    def v3(subject_cn, signer, issuer_name, san, ca=None,
           nb=datetime.datetime(2020, 1, 1), na=datetime.datetime(2999, 1, 1), digest=hashes.SHA256()):
        n = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, subject_cn)])
        b = (x509.CertificateBuilder().subject_name(n).issuer_name(issuer_name)
             .public_key(ee_key.public_key()).serial_number(3)
             .not_valid_before(nb).not_valid_after(na))
        if ca is not None:
            b = b.add_extension(x509.BasicConstraints(ca=ca, path_length=None), critical=True)
        if san:
            b = b.add_extension(x509.SubjectAlternativeName([x509.DNSName(subject_cn)]), critical=False)
        return b.sign(signer, digest).public_bytes(serialization.Encoding.DER)

    case("v3_issued_san", v3("localhost", ca_key, ca_subject, True),
         "v3 with SAN, issued by the trusted CA", roots="ca.pem")
    case("v3_issued_wrong_host", v3("localhost", ca_key, ca_subject, True),
         "v3 with SAN localhost, host example.com", host="example.com", roots="ca.pem")
    self_name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "localhost")])
    for n, (ca, san) in {"v3_self_ca_san": (True, True), "v3_self_ca_cn_only": (True, False),
                         "v3_self_noca_cn_only": (None, False),
                         "v3_self_noca_san": (None, True)}.items():
        case(n, v3("localhost", ee_key, self_name, san, ca=ca), n + ", trusted directly")
    case("v3_self_expired", v3("localhost", ee_key, self_name, True,
                               nb=datetime.datetime(2000, 1, 1), na=datetime.datetime(2001, 1, 1)),
         "v3 self-signed, trusted directly, expired")
    ed = load_or_create_ed25519_key(os.path.join(out, "keys", "ed25519_ca.key"))
    ed_ca = ca_certificate(ed, "Blaze Ed25519 CA", None)
    case("v3_ed25519_ca", v3("localhost", ed, ed_ca.subject, True, digest=None),
         "v3 with SAN, issued by a trusted Ed25519 CA",
         roots=root_file("v3_ed25519_ca", ed_ca.public_bytes(serialization.Encoding.DER)))
    p256 = load_or_create_p256_key(os.path.join(out, "keys", "p256_ca.key"))
    p256_ca = ca_certificate(p256, "Blaze P-256 CA", hashes.SHA256())
    case("v3_p256_ca", v3("localhost", p256, p256_ca.subject, True),
         "v3 with SAN, issued by a trusted P-256 CA",
         roots=root_file("v3_p256_ca", p256_ca.public_bytes(serialization.Encoding.DER)))

    with open(os.path.join(out, "cases.tsv"), "w") as f:
        f.write("# case\troots\thost\tdescription\n")
        for row in cases:
            f.write("\t".join(row) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
