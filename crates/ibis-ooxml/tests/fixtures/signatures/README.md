# Synthetic signature fixtures

These fixtures contain self-signed RSA and EC test certificates and synthetic data.
The private keys were discarded. They do not represent trusted identities.

`rsa-sha256-synthetic.xml` tests signature-value verification in isolation.
`rsa-sha1-synthetic.xml` is an independently generated OpenSSL RSA-1024/SHA-1
fixture for weak legacy diagnostics only; its private key was discarded.
`legacy-signed.docx` is an independently signed synthetic package using the
same weak algorithms, for end-to-end diagnostic and tampering tests.
`synthetic-signed.docx` is a minimal OPC package for tests, not a Microsoft
Office interoperability fixture. Its SignedInfo was canonicalized with lxml,
signed with OpenSSL, and verified with OpenSSL. The included certificate has
a short validity period; the tests intentionally do not check certificate
validity or establish trust.

`signed-editable.docx` is a synthetic DOCX with one independently generated
RSA-SHA256 XML signature over `word/document.xml`. It uses a self-signed test
certificate and does not claim Microsoft Office signature interoperability.
Its private key was discarded after the signature was checked with OpenSSL.

`digital-signature.der` and `encipherment-only.der` are separate synthetic
OpenSSL certificates used to check KeyUsage decoding. Their private keys were
discarded and they are not trust anchors.

`issuer.der`, `other-issuer.der`, and `issuer-signed-leaf.der` form a
synthetic RSA certificate-link fixture. The two CA certificates have the same
subject name and distinct keys; OpenSSL verified the leaf against `issuer`.
All generated private keys were discarded.

`crl-issuer.der`, `crl-leaf.der`, `empty.crl.der`, and `revoked.crl.der`
were generated with OpenSSL for CRL inspection; OpenSSL verified the CRL's
signature. The certificates and CRLs are synthetic and short-lived.

`xades-signed-synthetic.docx` is an independently signed synthetic package
with XAdES v1.3.2 `SignedProperties`, generated with lxml and OpenSSL and
verified with OpenSSL. Its private key was discarded. It is deliberately
minimal and does not establish XAdES conformance or any trusted identity.

`xades-cert-synthetic.docx` adds a SHA-256 SigningCertificateV2 digest to a
synthetic package. Its SignedInfo was signed and independently checked using
OpenSSL; the private key was discarded. It is not an XAdES conformance fixture.

`ecdsa-p256-signed.docx` is a synthetic package signed with OpenSSL ECDSA
P-256/SHA-256 and independently verified with OpenSSL before conversion from
DER to XML Signature's fixed-width `r || s` format. `ecdsa-p256-signature.der`
and `ecdsa-p384-cert.der` test rejection of the wrong signature encoding and
unsupported curve. Both private keys were discarded.

`ec-ca.der`, `ec-other.der`, `ec-p384.der`, `ec-leaf.der`,
`ec-empty.crl.der`, and `ec-revoked.crl.der` are independently generated
OpenSSL EC certificates and CRLs. The two P-256 CAs have identical names and
distinct keys, and the P-384 CA uses the same name. OpenSSL verified the leaf
and both CRL signatures against `ec-ca.der`; all private keys were discarded.

`xades-issuer-serial-synthetic.docx` is a synthetic OpenSSL-signed package
with an independently encoded IssuerSerialV2. `xades-wrong-issuer.der`,
`xades-wrong-serial.der` and `xades-multiple-names.der` exercise the diagnostic
with nonmatching and unsupported inputs. The signing key was discarded.

`rsa-sha2-ca.der`, `rsa-sha384-leaf.der`, `rsa-sha512-leaf.der`, the
corresponding CRLs, and `rsa-sha384-signed.docx` / `rsa-sha512-signed.docx`
are synthetic OpenSSL-signed fixtures. OpenSSL verified each leaf, CRL, and
SignedInfo signature; the private keys were discarded.

`rsa-pss-sha256-signed.docx` is a synthetic OpenSSL-signed package using
RSA-PSS SHA-256, MGF1-SHA256, and a 32-byte salt. `rsa-pss-salt16.sig` and
`rsa-pss-mgf384.sig` sign the same canonicalized SignedInfo with unsupported
parameters. OpenSSL verified all three with their respective parameters, and
the private key was discarded.

`pss-ca.der`, `pss-other.der`, and `pss-{normal,salt20,mgf384}-leaf.der` /
`.crl.der` are synthetic Python cryptography-generated RSA certificates and
CRLs. The two CAs have the same subject and distinct keys. LibreSSL verified
all leaf and CRL signatures independently; the keys were discarded.

`rsa-pss-param-{default,empty,explicit}.docx` are synthetic RSA-PSS XML
signatures with omitted, empty, and explicitly specified parameters. OpenSSL
verified each SHA-256/MGF1-SHA256/32-byte-salt signature. The key was discarded.

`p384-ca.der`, `p384-other.der`, `p384-{normal,sha256}-leaf.der`,
`p384-{normal,sha256}.crl.der`, and `p384-signed-synthetic.docx` are
independently generated P-384 test data. LibreSSL verified each signature,
and `p384-signature.der` checks rejection of DER in XML SignatureValue.
The CA private keys were discarded.

`relationship-selection-signed.docx` is an independently generated synthetic
package with one selected and one unselected relationship. Its package object
and SignedInfo were canonicalized with lxml, signed with OpenSSL, and its
signature was verified with OpenSSL. The key was discarded.

`location-hints.der`, `location-hints-overflow.der`, and
`location-hints-signed.docx` are synthetic certificates and a signed package
with placeholder OCSP, CA-issuer, and CRL addresses under `.invalid`. The
package signature was independently checked with LibreSSL; the key was discarded.

`ocsp-*.der` are synthetic certificates and OCSP responses generated with
Python cryptography. LibreSSL independently verified the response signatures;
the private keys were discarded. The response signer is deliberately
untrusted and unauthorized, so a matching signature does not establish
responder authorization. The two issuer certificates have the same subject
and different keys.

`ocsp-request.der` is a synthetic SHA-256 CertID request with a fixed test
nonce generated independently with Python cryptography and inspected with
LibreSSL. The fixed nonce is for tests only.
