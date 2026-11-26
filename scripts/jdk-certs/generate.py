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

import datetime
import os
import sys

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import padding, rsa
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

    def case(case_name, der, description, host="localhost", roots="ca.pem"):
        with open(os.path.join(out, case_name + ".der"), "wb") as f:
            f.write(der)
        cases.append((case_name, roots, host, description))

    def root_file(case_name, *ders):
        path = case_name + ".root.pem"
        with open(os.path.join(out, path), "w") as f:
            for der in ders:
                f.write(x509.load_der_x509_certificate(der).public_bytes(
                    serialization.Encoding.PEM).decode())
        return path

    ee = lambda **kw: certificate(ca_key, ca_name, cn_name("localhost"), ee_key, **kw)

    case("v1", ee(), "v1, no version field, issued by the CA")
    case("v1_upper_host", ee(), "v1, host name in upper case", host="LOCALHOST")
    case("v1_explicit", ee(version=explicit_version(0)), "version [0] INTEGER 0 encoded")
    case("v2", ee(version=explicit_version(1)), "version 2, no unique identifiers")
    ext = tlv(0xA3, seq(seq(oid("2.5.29.19"), tlv(0x04, seq()))))
    case("v1_extensions", ee(trailing=ext), "Q1a: v1 (no version) with an extensions block")
    case(
        "v1_explicit_extensions",
        ee(version=explicit_version(0), trailing=ext),
        "Q1a: version 0 encoded, with an extensions block",
    )
    case("v2_extensions", ee(version=explicit_version(1), trailing=ext), "Q1a: v2 with extensions")
    uid1 = tlv(0x81, b"\x00\x01")
    uid2 = tlv(0x82, b"\x00\x02")
    case("v2_unique_ids", ee(version=explicit_version(1), trailing=uid1 + uid2),
         "v2 with issuerUniqueID and subjectUniqueID")
    case("v2_unique_ids_reversed", ee(version=explicit_version(1), trailing=uid2 + uid1),
         "v2 with subjectUniqueID before issuerUniqueID")
    case("v1_unique_id", ee(trailing=uid1), "v1 (no version) with an issuerUniqueID")
    case("v2_trailing_integer", ee(version=explicit_version(1), trailing=integer(b"\x01")),
         "v2 with an INTEGER after the key")
    case(
        "alg_mismatch",
        ee(inner_alg=alg(SHA384_RSA)),
        "Q1b: inner signature sha384WithRSA, outer (and signature) sha256WithRSA",
    )
    case(
        "alg_mismatch_outer",
        ee(outer_alg=alg(SHA384_RSA), digest=hashes.SHA384),
        "Q1b: inner sha256WithRSA, outer (and signature) sha384WithRSA",
    )
    case(
        "alg_params_absent_inner",
        ee(inner_alg=seq(oid(SHA256_RSA))),
        "inner sha256WithRSA without NULL parameters, outer with",
    )
    case("expired", ee(validity=EXPIRED), "issued by the CA, expired in 2001")
    case("wrong_host", ee(), "CN=localhost, host example.com", host="example.com")
    case("ip_host", certificate(ca_key, ca_name, cn_name("127.0.0.1"), ee_key),
         "CN=127.0.0.1, host 127.0.0.1", host="127.0.0.1")
    case("trailing_dot_host", ee(), "host localhost. (trailing dot)", host="localhost.")
    case("underscore", certificate(ca_key, ca_name, cn_name("local_host"), ee_key),
         "CN and host local_host", host="local_host")
    case("wildcard", certificate(ca_key, ca_name, cn_name("*.example.com"), ee_key),
         "CN=*.example.com, host foo.example.com", host="foo.example.com")
    two_cn_a = name([(CN, printable("localhost")), (CN, printable("other"))])
    two_cn_b = name([(CN, printable("other")), (CN, printable("localhost"))])
    case("two_cn_first", certificate(ca_key, ca_name, two_cn_a, ee_key),
         "Q4: one RDN, CN=localhost then CN=other (as encoded)")
    case("two_cn_second", certificate(ca_key, ca_name, two_cn_b, ee_key),
         "Q4: one RDN, CN=other then CN=localhost (as encoded)")
    case("cn_last_rdn", certificate(ca_key, ca_name,
                                    name([(CN, printable("other"))], [(O, printable("x"))],
                                         [(CN, printable("localhost"))]), ee_key),
         "CN=other, O=x, CN=localhost in three RDNs")
    case("cn_first_rdn", certificate(ca_key, ca_name,
                                     name([(CN, printable("localhost"))],
                                          [(CN, printable("other"))]), ee_key),
         "CN=localhost, CN=other in two RDNs")
    case("cn_bmp", certificate(ca_key, ca_name, name([(CN, bmp("localhost"))]), ee_key),
         "CN=localhost as a BMPString")
    case("empty_subject", certificate(ca_key, ca_name, seq(), ee_key), "v1, empty subject")
    case("empty_subject_v2", certificate(ca_key, ca_name, seq(), ee_key,
                                         version=explicit_version(1)), "v2, empty subject")
    case("empty_issuer", certificate(ca_key, seq(), cn_name("localhost"), ee_key),
         "v1, empty issuer")
    case("serial_leading_zero", ee(serial=b"\x00\x01"), "serial number 00 01 (not DER)")
    case("serial_negative", ee(serial=b"\xff"), "serial number -1")
    case("time_non_ascii", ee(validity=seq(utc("2\u00e9032620181Z".encode("utf-8")),
                                           gen("29990528201814Z"))),
         "Q2: notBefore UTCTime of 12 bytes with a two-byte UTF-8 character at bytes 1-2")
    case("time_no_seconds", ee(validity=seq(utc("2003262018Z"), gen("29990528201814Z"))),
         "UTCTime without seconds")
    case("time_offset", ee(validity=seq(utc("200326201814+0100"), gen("29990528201814Z"))),
         "UTCTime with a +hhmm offset")
    case("time_fraction", ee(validity=seq(utc("200326201814Z"), gen("29990528201814.5Z"))),
         "GeneralizedTime with a fraction")
    case("time_feb_31", ee(validity=seq(utc("200231201814Z"), gen("29990528201814Z"))),
         "notBefore 2020-02-31")
    case("time_second_60", ee(validity=seq(utc("200326201860Z"), gen("29990528201814Z"))),
         "notBefore second 60")
    long_issuer = ca_name[:1] + b"\x81" + ca_name[1:] if ca_name[1] < 0x80 else None
    if long_issuer is not None:
        case("non_minimal_length",
             certificate(ca_key, long_issuer, cn_name("localhost"), ee_key),
             "issuer Name length in long form (not DER)")
    case("trailing_bytes", ee(after_cert=b"\x00\x00"), "two bytes after the certificate")
    case("version_5", ee(version=explicit_version(5)), "version field 5")

    # Q3: trusted directly.
    other_name = cn_name("Other Issuer")
    direct = certificate(other_key, other_name, cn_name("localhost"), ee_key)
    case("direct", direct, "Q3: v1 issued by an unknown issuer, itself the trust anchor",
         roots=root_file("direct", direct))
    direct_expired = certificate(other_key, other_name, cn_name("localhost"), ee_key,
                                 validity=EXPIRED)
    case("direct_expired", direct_expired,
         "S4: as direct, expired", roots=root_file("direct_expired", direct_expired))
    self_signed = certificate(ee_key, cn_name("localhost"), cn_name("localhost"), ee_key)
    case("self_signed", self_signed, "self-signed v1, itself the trust anchor",
         roots=root_file("self_signed", self_signed))
    self_signed_expired = certificate(ee_key, cn_name("localhost"), cn_name("localhost"),
                                      ee_key, validity=EXPIRED)
    case("self_signed_expired", self_signed_expired, "S4: self-signed v1, trusted, expired",
         roots=root_file("self_signed_expired", self_signed_expired))
    case("self_signed_untrusted", self_signed, "self-signed v1, not in the trust store")
    case("bad_signature", certificate(other_key, ca_name, cn_name("localhost"), ee_key),
         "issuer name of the CA, signed by another key")

    with open(os.path.join(out, "cases.tsv"), "w") as f:
        f.write("# case\troots\thost\tdescription\n")
        for row in cases:
            f.write("\t".join(row) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
