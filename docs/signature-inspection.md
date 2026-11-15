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
