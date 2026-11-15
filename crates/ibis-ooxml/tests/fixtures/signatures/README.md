# Synthetic signature fixtures

These fixtures contain self-signed RSA test certificates and synthetic data.
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
