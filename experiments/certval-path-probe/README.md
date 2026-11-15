# Offline certificate path experiment

This separate Cargo workspace evaluates `certval` 0.2.0-rc.3 with its default
features disabled and its `rsa` feature enabled. The experiment depends on
`ibis-ooxml`, but it is **not linked into the ibisOffice product**, is not a
production trust decision, and does not use pverify.
No system trust store, trust-list provider, network retrieval, OCSP or CRL
checking is enabled. Each evaluated path ends at a caller-supplied anchor. The
explicit-path tests supply a constructed path, while the document bridge below
also uses ibis-ooxml's bounded candidate search. The reference time is the explicit test value `1796083200` (2026-12-01 UTC).
The caller must not substitute an untrusted document certificate as an anchor. `set_require_ta_store(false)` is
used only because the test itself supplies the anchor; no trust-store membership
check is performed.

From the repository root:

```sh
cargo test --locked --manifest-path experiments/certval-path-probe/Cargo.toml -- --nocapture
cargo +nightly-2026-09-17 check --locked \
  --manifest-path experiments/certval-path-probe/Cargo.toml --target wasm32-unknown-unknown
python3 experiments/certval-path-probe/verify_policy_reference.py
python3 experiments/certval-path-probe/verify_algorithm_reference.py
```

Two tests exercise 11 combinations: a direct valid path, an existing
RSA-SHA384 fixture, a path with an explicit intermediate, a different issuer,
a same-name/different-key issuer, an expired target, a changed signature,
an unknown critical extension, a missing intermediate, a non-CA intermediate,
and an intermediate without keyCertSign. Rejection tests check their respective
error categories. The DER files under `data/` were created with Python
cryptography for this experiment; keys were discarded. Other certificates
are existing independently created ibis-ooxml synthetic fixtures. All OIDs and
names are synthetic.

A third test uses a separate synthetic chain and nine policy configurations:
matching, mismatching and absent policies with explicit policy required or not;
`anyPolicy` allowed or inhibited; and policy mapping allowed or inhibited.
The same nine combinations were independently checked with LibreSSL `openssl
verify` using `-policy 1.2.3.4`, `-explicit_policy`, `-inhibit_any` and
`-inhibit_map` as applicable, and their accept/reject outcomes matched. In these
fixtures, specifying an initial policy alone still accepts a mismatch or absent
policy when explicit policy is not required. A caller must choose these settings
as part of its trust policy; the probe does not infer them from a certificate.
The policy fixtures were independently generated with Python cryptography and
the private keys were discarded. These results cover only these fixtures and
settings, not general policy-tree conformance. The reference script uses the host
clock and requires it to be within the synthetic certificates' 2026–2029 validity
period; the Rust probe uses the fixed time above.

A fourth test exercises existing synthetic P-256/SHA-256, P-384/SHA-384,
P-384/SHA-256 and RSA-PSS signatures, including different PSS salt lengths.
A matching independent LibreSSL check accepts all six intact signatures and
rejects a wrong issuer and a damaged signature. With SHA-256 as the signature
hash and MGF1/SHA-384, LibreSSL accepts the signature but certval 0.2.0-rc.3
returns `SignatureVerificationFailure`. Its [crypto callback](https://docs.rs/crate/certval/0.2.0-rc.3/source/src/util/crypto.rs)
reports this MGF combination as unrecognized, while its [path validator](https://docs.rs/crate/certval/0.2.0-rc.3/source/src/validator/path_validator.rs)
converts signature-verification errors to the same path status used for a bad
signature. This is fail-closed, but the path result alone cannot distinguish an
unsupported parameter from a cryptographic mismatch. Any integration needs an
explicit indeterminate/unsupported outcome and must not label that result as
proof of a bad signature. The reference script uses the host clock and requires
it to be in the fixtures' common validity window, 2026-11-15 01:22:06 UTC to
2027-11-15 01:22:06 UTC. The Rust probe uses its fixed reference time.

A separate single-edge diagnostic calls the crypto interface directly with the
original signed DER bytes and an explicitly proposed issuer certificate. For
the mixed-hash PSS fixture it preserves `Unrecognized`, whereas a damaged
supported PSS signature produces the backend's signature-failure result.
Damaging the unsupported signature still yields `Unrecognized`, so that result
says nothing about whether that signature is intact. The backend can also
report a signature failure for other unsupported inputs, so this distinction is
not a complete algorithm-support classifier. The diagnostic reports malformed
inputs and other errors separately and never changes the path outcome.

A separate test demonstrates why the diagnostic is not path validation: an
end-entity signature matches a proposed issuer key while the corresponding
path fails because the proposed intermediate is not a CA. The diagnostic does
not authenticate the issuer, check its name, trust, validity, revocation or
purpose, or validate any other path edge. The adapter remains experimental and
is not exposed to the application. It must not turn `Matches` into a trust
or path success result.

A bounded path-report experiment records both the unchanged engine result and
an ordered list of independent signature observations, from the supplied anchor
toward the target. It accepts at most seven supplied intermediates, each
certificate at most 1 MiB, and at most 4 MiB total. These limits apply to this
probe and are not a general resource audit. Tests retain the engine failure
when all edge signatures match but an intermediate is not a CA or the target
has an unprocessed critical extension. They also show that the same engine
signature-failure result can accompany either an unsupported edge or a backend
signature failure. No edge observation overrides the path result, and the path
result is not a revocation or document-signature verdict. The experiment takes
the caller's path, time and settings; it does not choose anchors, construct
paths, or establish the provenance of any input.

The document bridge starts from a relationship-discovered OPC signature and
its single embedded certificate, and supplies only caller-provided intermediate
and anchor certificates to ibis-ooxml's candidate search. It keeps the XML
signature value, package-object digest, individual manifest-reference digests,
and every candidate's path and edge observations separate. A candidate whose
anchor is byte-identical to the signer is reported as not evaluated. Candidate
certificates are resolved by SHA-256 against the original inputs; distinct
byte strings sharing a fingerprint cause an error. The existing candidate
search uses exact names, is bounded, and can omit valid paths; an empty or
truncated search is not a trust or invalidity verdict. It does not use
certificate location hints or fetch data.

The synthetic `document-path-signed.docx` and its test chain were generated
independently with Python cryptography and lxml; LibreSSL verified the XML
signature over canonical SignedInfo and the certificate chain. Keys were
discarded. One test gets matching XML signature, object digest, manifest digest
and a passing candidate path; another changes the document part and finds the
manifest digest differs while the other observations remain unchanged. Neither
result establishes complete coverage, a valid timestamp, revocation status,
anchor provenance, or document-signature validity. Other SignedInfo references,
coverage and application-specific rules still need separate evaluation.

The pinned crate uses Apache-2.0 OR MIT licensing. Its own
[security warning](https://docs.rs/crate/certval/0.2.0-rc.3/source/README.md)
says it has not been independently audited and documents remaining corpus
mismatches, including CRL cases. A path success here says nothing about
revocation, trusted-list authenticity, document signature validity, timestamp,
coverage, qualified status, FPKI, eIDAS or Japanese PKI compliance. The Wasm
command checks compilation only. These few synthetic cases are not a standards
or security qualification and do not establish production readiness.

Before connecting a path result to a document signature, a separate design and
broader tests must cover explicit anchor selection and provenance, path-building
limits, policy inputs and mappings, signer key usage and purpose, historical
time and trusted timestamps, offline revocation and freshness, unsupported
algorithms and critical extensions, resource limits, and distinct indeterminate
versus rejection results. Any platform and trust-community decisions must be
made explicitly; no default trust stores or PKI membership are inferred here.
