# Office package signature inspection

The experimental inspector reports observations about OPC package signatures:

```sh
cargo run --locked -p ibis-ooxml --bin ibis-signature-inspect -- document.docx
```

It accepts a DOCX, XLSX, or PPTX package up to 128 MiB. It reports the
signature-value comparison against an embedded **untrusted** certificate, the
package-object digest, and the manifest reference digest and content-type
comparisons. Signature-value comparisons support RSA PKCS#1 v1.5 with SHA-256/384/512,
RSA-PSS SHA-256 with MGF1-SHA256 and a 32-byte salt, ECDSA P-256/SHA-256
and P-384/SHA-384, and weak legacy RSA-SHA1. RSA-PSS supports the parameterless
`sha256-rsa-MGF1` URI and the `rsa-pss` URI with the same parameters, whether
omitted or explicitly encoded in the supported subset. Other parameter
combinations and restricted PSS public keys are not checked. ECDSA uses the
XML Signature fixed-width `r || s` encoding; other curves and algorithms are unsupported. These checks
use an untrusted embedded certificate and do not validate certificate chains.
Unknown algorithms and transformations are reported as unsupported.
Additional SignedInfo references to direct `ds:Object` children, and to
XAdES v1.3.2 `SignedProperties` inside `QualifyingProperties` associated with
the signature, can be compared by a simple ASCII ID using SHA-256/384/512 or
legacy SHA-1 and inclusive canonicalization. This is a digest observation; it
does not validate the XAdES properties, signing-certificate binding, XAdES
conformance or eIDAS qualification. For a supported `SignedProperties`, the
first `SigningCertificateV2` certificate digest is also compared with the
single embedded certificate, using SHA-256/384/512 or legacy SHA-1. This is an
unauthenticated comparison: the property reference, signature value and
certificate must be considered separately. If the first reference has an
`IssuerSerialV2`, its serial and a single directoryName are compared against
the embedded certificate. The name comparison is exact DER equality, not RFC
5280 semantic name matching; other name forms are unsupported. Remaining
certificate references and certificate trust are not checked. See [ETSI EN 319 132-1](https://www.etsi.org/deliver/etsi_en/319100_319199/31913201/01.03.01_60/en_31913201v010301p.pdf). See [ETSI TS 101 903](https://www.etsi.org/deliver/etsi_ts/101900_101999/101903/01.04.02_60/ts_101903v010402p.pdf). For same-document shortname fragments, comments are omitted
as specified by [XML Signature 1.1](https://www.w3.org/TR/xmldsig-core1/#sec-Same-Document).
Other references, including external or other nested targets and
unsupported transformations, are reported as not checked and are never fetched.
The inspector rejects more than 256 additional references or an additional URI
longer than 4096 bytes, and limits total object bytes processed to 16 MiB.

For each signature, an inventory compares non-directory package entry names with
literal, directly recognizable package-manifest reference paths. The CLI shows
up to 16 shortened examples of unlisted entries; the desktop dialog shows
counts. Percent-encoded or otherwise ambiguous paths are counted separately.
This is not a coverage verdict: the inventory includes metadata and signature
parts; other references or signatures may apply, and a relationship transform
can select only some relationships from a listed entry. Listed entries are not
necessarily fully protected, and unlisted entries are not necessarily unsigned
or relevant to rendered content. See the [OPC signature overview](https://learn.microsoft.com/en-us/previous-versions/windows/desktop/opc/digital-signatures-overview).

For supported relationship transforms, each manifest reference also reports
how many relationships currently match the declared `SourceId` or `SourceType`
selectors, how many do not, and how many distinct selectors currently match
nothing. The CLI shows up to eight IDs in each group; the desktop dialog gives
per-reference totals, which can count a relationship more than once. These are
unauthenticated current-state observations, not a coverage decision. Raw-part
references and unsupported relationship transforms are reported separately.
This inspector applies either selector form; [Microsoft documents](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/e8e2269c-d849-456c-a037-d54a3bf0aaa8)
that Office uses `SourceId` when applying the transform, so a `SourceType`
observation does not establish Office interoperability.

**This is not signature validation.** Certificate path and trust, revocation,
timestamps, unsupported SignedInfo references, complete document coverage, and
the effect of later edits are not established. A matching digest or signature
value alone does not mean that a document or signer is trustworthy. A signature
may cover only selected parts of a document.

On native file open, ibisOffice warns if signature parts are found or signature
discovery fails. Before saving an edited signed or uninspectable package through
the native Save, Save As, or save-on-close actions, the app asks for confirmation.
Copy and export actions also ask, because their output may not retain a signature.
These warnings do not verify signatures and are not yet wired to browser-based
document opening.

On desktop, File > Inspect Digital Signatures shows the same limited
observations for the package originally opened. The results do not reflect
later changes or saves; reopen a file to inspect that file's bytes.

Exit status 0 means inspection completed; it is not a validity verdict. Exit
status 2 means the input could not be inspected. The command does not fetch
certificates or revocation information and does not modify the input file.

The inspector also reads location hints from the single embedded signing
certificate's AIA and CRL distribution-point extensions. It distinguishes
OCSP, CA-issuer and CRL URI counts, non-URI or unsupported forms, critical
extensions, and CRL reason restrictions or alternate issuers. The CLI prints
at most eight complete URI examples per category, each at most 2048 bytes;
the desktop dialog shows counts. These strings are untrusted certificate
contents and are not fetched, validated as URLs, or treated as proof of issuer
identity, CRL applicability or revocation status. Intermediate certificates,
Freshest CRL and other access methods are not processed as revocation sources.

Optionally, supply an explicit Unix reference time and DER certificates:

```sh
cargo run --locked -p ibis-ooxml --bin ibis-signature-inspect -- document.docx \
  --at 1794614400 --anchor root.der --intermediate intermediate.der
```

`--at` reports the embedded certificate's time window and key usage. `--anchor`
and `--intermediate` can be repeated and enumerate bounded candidate chains.
These candidates use exact ASN.1 name matching and are **not validated**;
signature mismatches may appear among them. Individual certificate and CRL
signature comparisons support RSA PKCS#1 v1.5 with SHA-256/384/512,
RSA-PSS SHA-256 with explicitly encoded MGF1-SHA256 and 32-byte salt, and
ECDSA P-256/SHA-256 and P-384/SHA-384 with DER-encoded ECDSA signatures.
Restricted PSS public keys and other PSS parameter combinations are not checked. Other algorithms
and curves are not checked. A zero-candidate result does not establish that no
path exists. DER inputs are supplied explicitly; the tool
does not use the system trust store.

To inspect a caller-supplied CRL against the embedded certificate, specify all
three inputs (`--crl` can be repeated):

```sh
cargo run --locked -p ibis-ooxml --bin ibis-signature-inspect -- document.docx \
  --at 1794614400 --crl revocations.der --crl-issuer issuer.der
```

The CRL output includes signature, issuer-name, update-window and serial-list
observations. It does not establish issuer trust or CRL applicability. A serial
being absent does not establish that a certificate is not revoked. Scoped,
indirect, delta and unrecognized CRL extensions remain unsupported.

To inspect a caller-supplied OCSP response, provide an explicit time, the
certificate issuer and the response signer certificate. `--ocsp` can be repeated:

```sh
cargo run --locked -p ibis-ooxml --bin ibis-signature-inspect -- document.docx \
  --at 1794614400 --ocsp response.der --ocsp-issuer issuer.der \
  --ocsp-responder responder.der --ocsp-nonce request-nonce.bin
```

The offline observations separate the response signature and responder ID from
CertID name, key and serial comparisons, claimed status, and response times.
The SHA-1 used in some identifiers is a protocol identifier, not a signature
or trust decision. The signer certificate is supplied by the caller; embedded
response certificates are counted but not selected automatically. At most 64
individual responses are reported, with truncation indicated. Unsupported
response types, hash algorithms and signature algorithms are not evaluated.
Optionally, `--ocsp-nonce` supplies the expected raw nonce bytes from the
caller (1 to 32 bytes, not DER or hex). RFC 8954 specifies 32 bytes for new
clients; the inspector accepts shorter values for observation and does not
generate nonces. The same bytes are compared with each
supplied response. The report distinguishes absent, duplicate, malformed,
out-of-range, matching, differing, and not-compared values; it does not print
the nonce bytes. It does not create a request or prove the supplied nonce was
used in one, and cannot establish its randomness or freshness. A match in an
unauthenticated or otherwise unacceptable response is not replay protection.
See [RFC 6960](https://www.rfc-editor.org/rfc/rfc6960) and
[RFC 8954](https://www.rfc-editor.org/rfc/rfc8954.html).

Alternatively, provide a saved DER OCSP request with `--ocsp-request request.der`
in place of `--ocsp-nonce`. For each supplied response, the inspector reports
the request nonce and its comparison to the response nonce, plus the number
of exactly matching response CertIDs for each request entry. Exact matching
includes the encoded algorithm identifier; semantically equivalent but
structurally different identifiers may not match. Comparisons inspect at most
64 requests and 64 responses, and report truncation; zero or partial counts
are not proof that an identifier is absent. Request and response extension
presence and the presence of a request signature are reported, but request
signature and identity, request provenance, extension semantics, and request
or response acceptance are not verified. A request file does not establish
that the nonce was freshly generated or that the request was sent.

All status claims remain unauthenticated observations, including `Good`.
The supplied responder certificate is also inspected for byte equality with
the supplied issuer, a proposed issuer link, validity dates at both the explicit
reference time and `producedAt`, digital-signature key usage, and the specific
OCSP-signing EKU. Malformed or duplicate usage extensions are reported as
invalid; these individual observations do not establish delegation. Responder
authorization, certificate paths and validity, nonce policy, other extension semantics,
clock skew and freshness policy are not validated. The
output does not establish revocation status, trust, or legal validity and no
network access is performed.

An external interoperability probe used the Apache POI test documents at
[commit 47dcff769bb395034dddfd59c4609659b6dbc6a4](https://github.com/apache/poi/tree/47dcff769bb395034dddfd59c4609659b6dbc6a4/test-data/xmldsign).
DOCX, XLSX, PPTX, and a two-signature DOCX were detected. The signature values,
package-object digests, and all 8, 9, 31, and 16 manifest references,
respectively, matched in those samples using SHA-1 and 1024-bit RSA. These weak
legacy algorithms are reported separately as diagnostics and do not establish
security, identity, certificate trust, complete coverage, or legal validity.
Each signature also declares an additional `#idOfficeObject` SignedInfo
reference; its weak SHA-1 digest matched in each sample. These samples do not
establish general Office interoperability. A temporary copy
of the DOCX with two independently computed SHA-256 relationship digests was
used to check the 2006 Relationship Transform URI; that modified copy no longer
has its original signature and is not evidence of Office interoperability.
