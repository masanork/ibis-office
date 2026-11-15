# Office package signature inspection

The experimental inspector reports observations about OPC package signatures:

```sh
cargo run --locked -p ibis-ooxml --bin ibis-signature-inspect -- document.docx
```

It accepts a DOCX, XLSX, or PPTX package up to 128 MiB. It reports the
signature-value comparison against an embedded **untrusted** certificate, the
package-object digest, and the manifest reference digest and content-type
comparisons. Unknown algorithms and transformations are reported as unsupported.
Additional SignedInfo references are listed by the CLI and counted in the
desktop dialog; their targets and digests are not checked or fetched. The
inspector rejects more than 256 additional references or an additional URI
longer than 4096 bytes.

**This is not signature validation.** Certificate path and trust, revocation,
timestamps, additional SignedInfo references, complete document coverage, and
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
signature mismatches may appear among them. A zero-candidate result does not
establish that no path exists. DER inputs are supplied explicitly; the tool
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
reference, whose digest is not checked. These samples do not establish general
Office interoperability. A temporary copy
of the DOCX with two independently computed SHA-256 relationship digests was
used to check the 2006 Relationship Transform URI; that modified copy no longer
has its original signature and is not evidence of Office interoperability.
